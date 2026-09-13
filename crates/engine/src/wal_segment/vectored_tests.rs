use super::write_frame;
use std::collections::VecDeque;
use std::io::{self, ErrorKind, IoSlice, Write};

enum Action {
    Limit(usize),
    Interrupt,
    Fail,
}

#[derive(Default)]
struct ScriptedWriter {
    actions: VecDeque<Action>,
    bytes: Vec<u8>,
    calls: usize,
}

impl Write for ScriptedWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        panic!("the frame helper must use vectored writes");
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.calls += 1;
        let limit = match self.actions.pop_front() {
            Some(Action::Limit(n)) => n,
            Some(Action::Interrupt) => return Err(ErrorKind::Interrupted.into()),
            Some(Action::Fail) => return Err(ErrorKind::PermissionDenied.into()),
            None => usize::MAX,
        };
        let before = self.bytes.len();
        // Independent reference: copy the requested concatenation's prefix.
        self.bytes
            .extend(bufs.iter().flat_map(|buf| buf.iter()).take(limit));
        Ok(self.bytes.len() - before)
    }

    fn flush(&mut self) -> io::Result<()> {
        panic!("sync remains the append caller's responsibility");
    }
}

#[test]
fn complete_frame_uses_one_vectored_call() {
    let mut writer = ScriptedWriter::default();
    write_frame(&mut writer, b"header", b"payload", b"crc").unwrap();
    assert_eq!(writer.bytes, b"headerpayloadcrc");
    assert_eq!(writer.calls, 1);
}

#[test]
fn every_two_short_write_boundary_preserves_bytes() {
    for lengths in [[3, 7, 4], [3, 0, 4], [0, 7, 0], [0, 0, 0]] {
        let parts: Vec<Vec<u8>> = lengths
            .into_iter()
            .enumerate()
            .map(|(part, len)| vec![part as u8 + 1; len])
            .collect();
        let expected = parts.concat();
        if expected.is_empty() {
            let mut writer = ScriptedWriter::default();
            write_frame(&mut writer, &parts[0], &parts[1], &parts[2]).unwrap();
            assert_eq!(writer.calls, 0);
            continue;
        }
        for first in 1..=expected.len() {
            for second in 1..=expected.len() {
                let mut writer = ScriptedWriter {
                    actions: [Action::Limit(first), Action::Limit(second)].into(),
                    ..Default::default()
                };
                write_frame(&mut writer, &parts[0], &parts[1], &parts[2]).unwrap();
                assert_eq!(writer.bytes, expected, "{lengths:?}: {first}, {second}");
            }
        }
    }
}

#[test]
fn interruptions_preserve_every_unwritten_suffix() {
    let expected = b"headerpayloadcrc";
    for prefix in 1..expected.len() {
        let mut writer = ScriptedWriter {
            actions: [
                Action::Interrupt,
                Action::Limit(prefix),
                Action::Interrupt,
                Action::Interrupt,
            ]
            .into(),
            ..Default::default()
        };
        write_frame(&mut writer, b"header", b"payload", b"crc").unwrap();
        assert_eq!(writer.bytes, expected);
        assert_eq!(writer.calls, 5);
    }
}

#[test]
fn zero_progress_and_errors_stop_at_every_prefix() {
    let expected = b"headerpayloadcrc";
    for prefix in 0..expected.len() {
        for zero in [false, true] {
            let mut writer = ScriptedWriter::default();
            if prefix != 0 {
                writer.actions.push_back(Action::Limit(prefix));
            }
            writer
                .actions
                .push_back(if zero { Action::Limit(0) } else { Action::Fail });
            // A retry after this terminal error would incorrectly consume this.
            writer.actions.push_back(Action::Limit(usize::MAX));
            let error = write_frame(&mut writer, b"header", b"payload", b"crc").unwrap_err();
            assert_eq!(
                error.kind(),
                if zero {
                    ErrorKind::WriteZero
                } else {
                    ErrorKind::PermissionDenied
                }
            );
            assert_eq!(writer.bytes, expected[..prefix]);
            assert_eq!(writer.actions.len(), 1);
        }
    }
}

#[test]
fn scalar_fallback_with_one_byte_writes_preserves_frame() {
    #[derive(Default)]
    struct Scalar(Vec<u8>);
    impl Write for Scalar {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(byte) = bytes.first() {
                self.0.push(*byte);
                Ok(1)
            } else {
                Ok(0)
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            unreachable!()
        }
    }
    let mut writer = Scalar::default();
    write_frame(&mut writer, b"header", b"", b"crc").unwrap();
    assert_eq!(writer.0, b"headercrc");
}
