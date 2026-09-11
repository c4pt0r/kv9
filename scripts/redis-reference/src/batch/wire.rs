//! Bounded RESP2 GET/MGET/MSET exchange; no command replay or pipelining.
use super::model::{Config, ReadApi, MAX_WIRE};
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
    Value(Option<Vec<u8>>),
    Values(Vec<Option<Vec<u8>>>),
    Applied,
}
pub struct Operation {
    pub read: bool,
    pub read_api: ReadApi,
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
        // Setup always uses MGET/MSET, regardless of the measured read API.
        Self {
            read,
            read_api: ReadApi::Mget,
            keys,
            values,
        }
    }
}
pub fn encode(op: &Operation) -> Result<Vec<u8>, Failure> {
    if op.keys.is_empty()
        || op.keys.len() > 256
        || (!op.read && op.keys.len() != op.values.len())
        || (op.read && !op.values.is_empty())
        || (op.read_api == ReadApi::Get && (!op.read || op.keys.len() != 1))
    {
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
    let command = match (op.read, op.read_api) {
        (true, ReadApi::Get) => b"GET".as_slice(),
        (true, ReadApi::Mget) => b"MGET",
        (false, _) => b"MSET",
    };
    bulk(&mut bytes, command)?;
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
    read_api: ReadApi,
    count: usize,
    max_value: usize,
) -> Result<Reply, Failure> {
    if !(1..=256).contains(&count)
        || max_value > MAX_WIRE
        || (read_api == ReadApi::Get && (!read || count != 1))
    {
        return Err(Failure::Protocol);
    }
    let mut remaining = MAX_WIRE;
    let header = line(reader, &mut remaining).await?;
    if !read {
        return if header == b"+OK\r\n" {
            Ok(Reply::Applied)
        } else {
            Err(Failure::Protocol)
        };
    }
    if read_api == ReadApi::Get {
        return bulk_value(reader, &mut remaining, header, max_value)
            .await
            .map(Reply::Value);
    }
    if header != format!("*{count}\r\n").as_bytes() {
        return Err(Failure::Protocol);
    }
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let header = line(reader, &mut remaining).await?;
        values.push(bulk_value(reader, &mut remaining, header, max_value).await?);
    }
    Ok(Reply::Values(values))
}

async fn bulk_value(
    reader: &mut (impl AsyncRead + Unpin),
    remaining: &mut usize,
    header: Vec<u8>,
    max_value: usize,
) -> Result<Option<Vec<u8>>, Failure> {
    if header == b"$-1\r\n" {
        return Ok(None);
    }
    // Populated benchmark values have exactly the configured size.
    if header != format!("${max_value}\r\n").as_bytes() || max_value + 2 > *remaining {
        return Err(Failure::Protocol);
    }
    *remaining -= max_value + 2;
    let mut value = vec![0; max_value + 2];
    reader.read_exact(&mut value).await?;
    if !value.ends_with(b"\r\n") {
        return Err(Failure::Protocol);
    }
    value.truncate(max_value);
    Ok(Some(value))
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
        let result = decode(
            connection,
            op.read,
            op.read_api,
            op.keys.len(),
            c.value_bytes,
        )
        .await?;
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
                read_api: ReadApi::Mget,
                keys: vec![b"a".to_vec(), b"a".to_vec()],
                values: vec![b"\0\r\n".to_vec(), b"x".to_vec()],
            };
            assert_eq!(
                encode(&op).unwrap(),
                b"*5\r\n$4\r\nMSET\r\n$1\r\na\r\n$3\r\n\0\r\n\r\n$1\r\na\r\n$1\r\nx\r\n"
            );
            let bytes = b"*3\r\n$3\r\none\r\n$-1\r\n$3\r\ntwo\r\n";
            match decode(&mut &bytes[..], true, ReadApi::Mget, 3, 3)
                .await
                .unwrap()
            {
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
                    decode(&mut &bytes[..], true, ReadApi::Mget, 1, 3).await,
                    Err(Failure::Protocol)
                ));
            }
            assert!(matches!(
                decode(&mut &b"-OOM rejected\r\n"[..], false, ReadApi::Mget, 1, 3).await,
                Err(Failure::ServerError)
            ));
            assert!(matches!(
                decode(&mut &b"+QUEUED\r\n"[..], false, ReadApi::Mget, 1, 3).await,
                Err(Failure::Protocol)
            ));
            assert!(matches!(
                decode(&mut &b"+OK\r\n"[..], false, ReadApi::Mget, 1, 3).await,
                Ok(Reply::Applied)
            ));
            assert!(matches!(
                decode(&mut &b"*1\r\n$3\r\na"[..], true, ReadApi::Mget, 1, 3).await,
                Err(Failure::Io)
            ));
        });
    }
    #[test]
    fn oversized_and_mismatched_requests_are_bounded() {
        assert!(encode(&Operation {
            read: false,
            read_api: ReadApi::Mget,
            keys: vec![vec![0; MAX_WIRE]],
            values: vec![vec![1; 16]]
        })
        .is_err());
        assert!(encode(&Operation {
            read: false,
            read_api: ReadApi::Mget,
            keys: vec![vec![1]],
            values: vec![]
        })
        .is_err());
    }

    fn get(key: &[u8]) -> Operation {
        Operation {
            read: true,
            read_api: ReadApi::Get,
            keys: vec![key.to_vec()],
            values: vec![],
        }
    }

    #[test]
    fn get_is_a_real_single_bulk_exchange_and_setup_remains_mget() {
        run(async {
            assert_eq!(
                encode(&get(b"a\0\r\n")).unwrap(),
                b"*2\r\n$3\r\nGET\r\n$4\r\na\0\r\n\r\n"
            );
            let mut v = super::super::model::tests::config_json();
            v["version"] = serde_json::json!(2);
            v["read_api"] = serde_json::json!("get");
            let c: Config = serde_json::from_value(v).unwrap();
            let sizes = c.validate().unwrap();
            let setup = Operation::new(&c, true, [0].into_iter(), 0);
            assert_eq!(setup.read_api, ReadApi::Mget);
            assert_eq!(encode(&setup).unwrap().len(), sizes.mget_request_bytes);
            assert_eq!(
                encode(&get(&c.key(0))).unwrap().len(),
                sizes.get_request_bytes.unwrap()
            );
            for (bytes, size, expected) in [
                (b"$3\r\na\0b\r\n".as_slice(), 3, Some(b"a\0b".to_vec())),
                (b"$-1\r\n", 3, None),
                (b"$0\r\n\r\n", 0, Some(vec![])),
            ] {
                match decode(&mut &bytes[..], true, ReadApi::Get, 1, size)
                    .await
                    .unwrap()
                {
                    Reply::Value(actual) => assert_eq!(actual, expected),
                    _ => panic!("GET was decoded as a batch reply"),
                }
            }
        });
    }

    #[test]
    fn get_rejects_array_replies_malformed_bulk_and_invalid_cardinality() {
        run(async {
            for bytes in [
                b"*1\r\n$3\r\nabc\r\n".as_slice(),
                b"+OK\r\n",
                b"$-2\r\n",
                b"$03\r\nabc\r\n",
                b"$2\r\nab\r\n",
                b"$3\r\nabcXX",
                b"$999999999\r\n",
            ] {
                assert!(
                    matches!(
                        decode(&mut &bytes[..], true, ReadApi::Get, 1, 3).await,
                        Err(Failure::Protocol)
                    ),
                    "accepted {bytes:?}"
                );
            }
            assert!(matches!(
                decode(&mut &b"$3\r\na"[..], true, ReadApi::Get, 1, 3).await,
                Err(Failure::Io)
            ));
            assert!(matches!(
                decode(&mut &b"-ERR rejected\r\n"[..], true, ReadApi::Get, 1, 3).await,
                Err(Failure::ServerError)
            ));
            assert!(matches!(
                decode(&mut &b"$3\r\nabc\r\n"[..], true, ReadApi::Mget, 1, 3).await,
                Err(Failure::Protocol)
            ));
            for count in [0, 2, 257] {
                assert!(matches!(
                    decode(&mut &b"$3\r\nabc\r\n"[..], true, ReadApi::Get, count, 3).await,
                    Err(Failure::Protocol)
                ));
                let mut op = get(b"a");
                op.keys = vec![b"a".to_vec(); count];
                assert!(encode(&op).is_err());
            }
            let mut op = get(b"a");
            op.values.push(b"unused".to_vec());
            assert!(encode(&op).is_err());
            op.read = false;
            assert!(encode(&op).is_err());
            assert!(matches!(
                decode(&mut &b"+OK\r\n"[..], false, ReadApi::Get, 1, 3).await,
                Err(Failure::Protocol)
            ));
        });
    }

    #[test]
    fn lost_get_reply_is_not_replayed_and_only_a_new_call_reconnects() {
        run(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let mut c: Config =
                    serde_json::from_value(super::super::model::tests::config_json()).unwrap();
                c.version = 2;
                c.read_api = Some(ReadApi::Get);
                c.address = listener.local_addr().unwrap();
                let first = get(&c.key(0));
                let second = get(&c.key(1));
                let expected = [encode(&first).unwrap(), encode(&second).unwrap()];
                let value = c.value(1, 0);
                let sent = value.clone();
                let peer = tokio::spawn(async move {
                    for (index, request) in expected.iter().enumerate() {
                        let (mut stream, _) = listener.accept().await.unwrap();
                        let mut actual = vec![0; request.len()];
                        stream.read_exact(&mut actual).await.unwrap();
                        assert_eq!(&actual, request, "failed logical call was replayed");
                        if index == 1 {
                            let mut reply = format!("${}\r\n", sent.len()).into_bytes();
                            reply.extend_from_slice(&sent);
                            reply.extend_from_slice(b"\r\n");
                            stream.write_all(&reply).await.unwrap();
                        }
                    }
                });
                let mut connection = None;
                let failed = call(&mut connection, &c, &first).await;
                assert!(matches!(failed.result, Err(Failure::Io)));
                assert_eq!(
                    (failed.connection_attempts, failed.command_attempts),
                    (1, 1)
                );
                assert!(connection.is_none());
                let successful = call(&mut connection, &c, &second).await;
                match successful.result {
                    Ok(Reply::Value(Some(actual))) => assert_eq!(actual, value),
                    _ => panic!("new logical GET did not reconnect correctly"),
                }
                assert_eq!(
                    (successful.connection_attempts, successful.command_attempts),
                    (1, 1)
                );
                peer.await.unwrap();
            })
            .await
            .expect("bounded owned peer did not finish");
        });
    }
}
