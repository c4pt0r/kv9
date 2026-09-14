//! One owned Linux process exercising the production segmented-WAL APIs.
//! The LD_PRELOAD observer lives only in this probe; there are no library hooks.
use std::ffi::{c_char, c_int, c_void, CString};
use std::path::Path;

use kv9_common::{metrics::WalIoMetrics, AppliedPosition};
use kv9_engine::wal_stream::{RecoveryPlan, SegmentedWal};
use kv9_engine::{ColumnFamily, Engine, MemEngine, ReplicatedEngine, WriteBatch};
use serde_json::json;

#[link(name = "dl")]
unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

type Phase = unsafe extern "C" fn(*const c_char) -> c_int;

fn observer() -> Phase {
    let name = CString::new("kv9_published_directory_phase").unwrap();
    // RTLD_DEFAULT resolves only the explicitly preloaded local observer.
    let address = unsafe { dlsym(std::ptr::null_mut(), name.as_ptr()) };
    assert!(
        !address.is_null(),
        "PUBLISHED_DIRECTORY_MISSING_SHIM: required fsync observer is absent"
    );
    // The frozen shim exports exactly this C signature; the runner pins its ELF.
    unsafe { std::mem::transmute::<*mut c_void, Phase>(address) }
}

fn phase(observer: Phase, value: &str) {
    let value = CString::new(value).unwrap();
    assert_eq!(unsafe { observer(value.as_ptr()) }, 0x504431);
}

fn position(index: u64) -> AppliedPosition {
    AppliedPosition { term: 2, index }
}

fn value(index: u64) -> Vec<u8> {
    let mut value = format!("published-directory-value-{index}").into_bytes();
    value.resize(128, b'x');
    value
}

fn batch(index: u64) -> WriteBatch {
    let mut batch = WriteBatch::new();
    for cf in ColumnFamily::ALL {
        batch.put(cf, index.to_be_bytes().to_vec(), value(index));
    }
    batch
}

fn recover(path: &Path, expected: &[u64]) -> SegmentedWal {
    let engine = MemEngine::new();
    let (wal, report) = RecoveryPlan::read(path)
        .unwrap()
        .recover(
            4096,
            WalIoMetrics::shared(),
            |checkpoint| {
                assert!(checkpoint.is_none(), "probe has no checkpoint premise");
                Ok(())
            },
            |batch, at| engine.write_applied(batch, at.expect("positioned probe record")),
        )
        .unwrap();
    assert_eq!(report.replayed_records, expected.len() as u64);
    for index in [1_u64, 7, 11, 21, 31] {
        for cf in ColumnFamily::ALL {
            assert_eq!(
                engine.get(cf, &index.to_be_bytes()).unwrap(),
                expected.contains(&index).then(|| value(index)),
                "recovery lost an acknowledged value or admitted an unacknowledged value"
            );
        }
    }
    assert_eq!(
        wal.applied_position(),
        expected.last().copied().map(position)
    );
    wal
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        3,
        "usage: published_directory_probe FRESH_DATA success|fault"
    );
    let directory = Path::new(&args[1]);
    let fault = match args[2].as_str() {
        "success" => false,
        "fault" => true,
        _ => panic!("invalid expected probe outcome"),
    };
    let observer = observer();
    phase(observer, "start");
    assert!(directory.is_absolute() && !directory.exists());
    std::fs::create_dir(directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let path = directory.join("catalog.wal");
    phase(observer, "create");
    let mut wal = SegmentedWal::create_new(&path, [71; 16], 4096, WalIoMetrics::shared()).unwrap();
    phase(observer, "seed");
    for index in [1, 7, 11] {
        wal.append(&batch(index), Some(position(index))).unwrap();
    }
    assert_eq!(wal.active_header().sequence, 1, "seed unexpectedly rotated");
    drop(wal);
    phase(observer, "recover-initial");
    let mut wal = recover(&path, &[1, 7, 11]);
    let topology_before = std::fs::read(&path).unwrap();
    phase(observer, "rotate-1");
    let rotated = wal.rotate();
    let rotation_error = rotated.as_ref().err().map(ToString::to_string);
    let mut fenced_write = None;
    let mut fenced_rotation = None;
    let recovered_before_new_write = if fault {
        assert!(
            rotation_error.as_ref().is_some_and(|error| error.contains("os error 5")),
            "PUBLISHED_DIRECTORY_EXPECTED_EIO: rotation did not return the selected EIO: {rotated:?}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), topology_before);
        phase(observer, "fence-check");
        let write = wal.append(&batch(21), Some(position(21)));
        let retry = wal.rotate();
        assert!(write.is_err(), "failed rotation acknowledged a new write");
        assert!(retry.is_err(), "failed rotation retained writer authority");
        fenced_write = write.err().map(|error| error.to_string());
        fenced_rotation = retry.err().map(|error| error.to_string());
        drop(wal);
        phase(observer, "recover-after-fault");
        wal = recover(&path, &[1, 7, 11]);
        phase(observer, "write-after-recovery");
        wal.append(&batch(21), Some(position(21))).unwrap();
        vec![1, 7, 11]
    } else {
        rotated.unwrap();
        assert_eq!(wal.active_header().sequence, 2);
        phase(observer, "write-after-rotate-1");
        wal.append(&batch(21), Some(position(21))).unwrap();
        phase(observer, "rotate-2");
        wal.rotate().unwrap();
        assert_eq!(wal.active_header().sequence, 3);
        phase(observer, "write-after-rotate-2");
        wal.append(&batch(31), Some(position(31))).unwrap();
        vec![1, 7, 11, 21, 31]
    };
    drop(wal);
    phase(observer, "recover-final");
    let final_values = if fault {
        vec![1, 7, 11, 21]
    } else {
        vec![1, 7, 11, 21, 31]
    };
    drop(recover(&path, &final_values));
    phase(observer, "done");
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    println!(
        "{}",
        json!({
            "complete": true, "process_id": std::process::id(),
            "process_start_ticks": fields[19].parse::<u64>().unwrap(),
            "boot_id": std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim(),
            "directory": directory, "expected_fault": fault, "rotation_error": rotation_error,
            "failed_rotation_write_acknowledged": fault.then_some(false),
            "fenced_write_error": fenced_write, "fenced_rotation_error": fenced_rotation,
            "old_acknowledged": [1, 7, 11], "recovered_before_new_write": recovered_before_new_write,
            "recovered_final": final_values, "production_api_only": true,
            "physical_power_loss_test": false
        })
    );
}
