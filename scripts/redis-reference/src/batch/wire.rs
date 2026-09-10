//! Bounded RESP2 MGET/MSET exchange; no command replay or pipelining.
use super::model::{Config, MAX_WIRE};
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

pub type Connection = BufReader<TcpStream>;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    Deadline,
    Io,
    ServerError,
    Protocol,
    DataIntegrity,
}
impl From<std::io::Error> for Failure {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}
impl Failure {
    pub fn fatal(self) -> bool {
        matches!(self, Self::Protocol | Self::DataIntegrity)
    }
    pub fn index(self) -> usize {
        match self {
            Self::Deadline => 1,
            Self::Io => 2,
            Self::ServerError => 3,
            Self::Protocol => 4,
            Self::DataIntegrity => 5,
        }
    }
}
pub enum Reply {
    Values(Vec<Option<Vec<u8>>>),
    Applied,
}
pub struct Operation {
    pub read: bool,
    pub keys: Vec<Vec<u8>>,
    pub values: Vec<Vec<u8>>,
}
impl Operation {
    pub fn new(c: &Config, read: bool, indices: impl Iterator<Item = usize>, nonce: u64) -> Self {
        let mut keys = Vec::new();
        let mut values = Vec::new();
        for i in indices {
            keys.push(c.key(i));
            if !read {
                values.push(c.value(i, nonce));
            }
        }
        Self { read, keys, values }
    }
}
pub fn encode(op: &Operation) -> Result<Vec<u8>, Failure> {
    if op.keys.is_empty() || op.keys.len() > 256 || (!op.read && op.keys.len() != op.values.len()) {
        return Err(Failure::Protocol);
    }
    let mut bytes =
        format!("*{}\r\n", 1 + op.keys.len() * if op.read { 1 } else { 2 }).into_bytes();
    fn bulk(out: &mut Vec<u8>, value: &[u8]) -> Result<(), Failure> {
        if value.len() > MAX_WIRE || out.len() + value.len() + 32 > MAX_WIRE + 32 {
            return Err(Failure::Protocol);
        }
        out.extend_from_slice(format!("${}\r\n", value.len()).as_bytes());
        out.extend_from_slice(value);
        out.extend_from_slice(b"\r\n");
        Ok(())
    }
    bulk(&mut bytes, if op.read { b"MGET" } else { b"MSET" })?;
    for (i, k) in op.keys.iter().enumerate() {
        bulk(&mut bytes, k)?;
        if !op.read {
            bulk(&mut bytes, &op.values[i])?;
        }
    }
    if bytes.len() > MAX_WIRE {
        return Err(Failure::Protocol);
    }
    Ok(bytes)
}
async fn line(
    reader: &mut (impl AsyncRead + Unpin),
    remaining: &mut usize,
) -> Result<Vec<u8>, Failure> {
    let mut line = Vec::new();
    loop {
        if *remaining == 0 || line.len() >= 512 {
            return Err(Failure::Protocol);
        }
        let b = reader.read_u8().await?;
        *remaining -= 1;
        line.push(b);
        if b == b'\n' {
            break;
        }
    }
    if !line.ends_with(b"\r\n") {
        return Err(Failure::Protocol);
    }
    if line.first() == Some(&b'-') {
        return Err(Failure::ServerError);
    }
    Ok(line)
}
pub async fn decode(
    reader: &mut (impl AsyncRead + Unpin),
    read: bool,
    count: usize,
    max_value: usize,
) -> Result<Reply, Failure> {
    let mut remaining = MAX_WIRE;
    let header = line(reader, &mut remaining).await?;
    if !read {
        return if header == b"+OK\r\n" {
            Ok(Reply::Applied)
        } else {
            Err(Failure::Protocol)
        };
    }
    if header != format!("*{count}\r\n").as_bytes() {
        return Err(Failure::Protocol);
    }
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let header = line(reader, &mut remaining).await?;
        if header == b"$-1\r\n" {
            values.push(None);
            continue;
        }
        // This benchmark's populated values have one configured exact size.
        if header != format!("${max_value}\r\n").as_bytes() || max_value + 2 > remaining {
            return Err(Failure::Protocol);
        }
        remaining -= max_value + 2;
        let mut value = vec![0; max_value + 2];
        reader.read_exact(&mut value).await?;
        if !value.ends_with(b"\r\n") {
            return Err(Failure::Protocol);
        }
        value.truncate(max_value);
        values.push(Some(value));
    }
    Ok(Reply::Values(values))
}
pub struct Call {
    pub result: Result<Reply, Failure>,
    pub elapsed_ns: u64,
    pub connection_attempts: u64,
    pub connection_failures: u64,
    pub command_attempts: u64,
}
pub async fn connect(c: &Config) -> Result<Connection, Failure> {
    let stream = TcpStream::connect(c.address).await?;
    stream.set_nodelay(true)?;
    Ok(BufReader::new(stream))
}
pub async fn call(conn: &mut Option<Connection>, c: &Config, op: &Operation) -> Call {
    let start = Instant::now();
    let mut connects = 0;
    let mut connect_failures = 0;
    let mut commands = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(c.deadline_ms);
    let work = async {
        if conn.is_none() {
            connects = 1;
            match connect(c).await {
                Ok(value) => *conn = Some(value),
                Err(error) => {
                    connect_failures = 1;
                    return Err(error);
                }
            }
        }
        let bytes = encode(op)?;
        let connection = conn.as_mut().unwrap();
        commands = 1;
        connection.get_mut().write_all(&bytes).await?;
        let result = decode(connection, op.read, op.keys.len(), c.value_bytes).await?;
        if !connection.buffer().is_empty() {
            return Err(Failure::Protocol);
        }
        Ok(result)
    };
    let result = match tokio::time::timeout_at(deadline, work).await {
        Ok(result) => result,
        Err(_) => {
            if connects > 0 && conn.is_none() {
                connect_failures = 1;
            }
            Err(Failure::Deadline)
        }
    };
    // Discard framing state after any failure. Only the next NEW logical call
    // may reconnect; this call is never sent again, even after a lost MSET reply.
    if result.is_err() {
        *conn = None;
    }
    Call {
        result,
        elapsed_ns: start.elapsed().as_nanos() as u64,
        connection_attempts: connects,
        connection_failures: connect_failures,
        command_attempts: commands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(f: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f);
    }
    #[test]
    fn resp_preserves_order_duplicates_and_binary_data() {
        run(async {
            let op = Operation {
                read: false,
                keys: vec![b"a".to_vec(), b"a".to_vec()],
                values: vec![b"\0\r\n".to_vec(), b"x".to_vec()],
            };
            assert_eq!(
                encode(&op).unwrap(),
                b"*5\r\n$4\r\nMSET\r\n$1\r\na\r\n$3\r\n\0\r\n\r\n$1\r\na\r\n$1\r\nx\r\n"
            );
            let bytes = b"*3\r\n$3\r\none\r\n$-1\r\n$3\r\ntwo\r\n";
            match decode(&mut &bytes[..], true, 3, 3).await.unwrap() {
                Reply::Values(v) => {
                    assert_eq!(v, vec![Some(b"one".to_vec()), None, Some(b"two".to_vec())])
                }
                _ => panic!("wrong response kind"),
            }
        });
    }
    #[test]
    fn malformed_counts_lengths_and_server_errors_cannot_succeed() {
        run(async {
            for bytes in [
                b"*2\r\n".as_slice(),
                b"*1\n",
                b"*1\r\n$999999999\r\n",
                b"*1\r\n$3\r\nabcXX",
                b"*1\r\n:0\r\n",
            ] {
                assert!(matches!(
                    decode(&mut &bytes[..], true, 1, 3).await,
                    Err(Failure::Protocol)
                ));
            }
            assert!(matches!(
                decode(&mut &b"-OOM rejected\r\n"[..], false, 1, 3).await,
                Err(Failure::ServerError)
            ));
            assert!(matches!(
                decode(&mut &b"+QUEUED\r\n"[..], false, 1, 3).await,
                Err(Failure::Protocol)
            ));
            assert!(matches!(
                decode(&mut &b"+OK\r\n"[..], false, 1, 3).await,
                Ok(Reply::Applied)
            ));
            assert!(matches!(
                decode(&mut &b"*1\r\n$3\r\na"[..], true, 1, 3).await,
                Err(Failure::Io)
            ));
        });
    }
    #[test]
    fn oversized_and_mismatched_requests_are_bounded() {
        assert!(encode(&Operation {
            read: false,
            keys: vec![vec![0; MAX_WIRE]],
            values: vec![vec![1; 16]]
        })
        .is_err());
        assert!(encode(&Operation {
            read: false,
            keys: vec![vec![1]],
            values: vec![]
        })
        .is_err());
    }
}
