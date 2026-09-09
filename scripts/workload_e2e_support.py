"""Bounded process-aware waits for the local workload acceptance fixture."""
import time


def wait(label, condition, nodes, seconds=45, workload=None):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        # Check before accepting a progress file: it may outlive its writer.
        if workload is not None:
            name, process = workload
            code = process.poll()
            if code is not None:
                raise RuntimeError(f"{name} exited {code} while waiting for {label}; see {name}.log and retained report")
        for node, process in nodes.items():
            if process.poll() is not None:
                raise RuntimeError(f"replica {node} exited; see its retained log")
        result = condition()
        if result:
            return result
        time.sleep(0.02)
    raise RuntimeError("timed out: " + label)
