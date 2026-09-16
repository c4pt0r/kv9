"""GDB-only, read-only snapshot of the first GET map in a pinned diagnostic ELF.

Offsets come from this x86-64 disassembly, not a stable Rust ABI. Independent
validation must accept every decoded key/value and tree edge before using it.
Debugger-affected timings are never performance evidence.
"""
import hashlib
import json
import os
from pathlib import Path
import struct
import traceback

import gdb


capture_finished = False


class Capture(gdb.Breakpoint):
    def stop(self):
        try:
            return self.snapshot()
        except BaseException:
            traceback.print_exc()
            raise

    def snapshot(self):
        global capture_finished
        self.enabled = False
        inferior = gdb.selected_inferior()
        frame = gdb.newest_frame()
        read = lambda address, size: bytes(inferior.read_memory(address, size))
        u64 = lambda address: struct.unpack("<Q", read(address, 8))[0]
        reg = lambda name: int(frame.read_register(name))
        map_address = reg("rsi")
        operation = read(reg("rdx"), reg("rcx")).decode()
        query = read(reg("r8"), reg("r9"))
        assert operation == "get_hit" and len(query) == 27
        assert read(map_address, 1) == b"\x01" and u64(map_address + 16) == 4096
        root = u64(map_address + 8)
        nodes, seen = [], set()
        pending = [(root, "")]
        while pending:
            address, path = pending.pop()
            assert address not in seen and len(seen) < 4096 and len(path) < 64
            seen.add(address)
            # The erased pointer addresses Node data, after ArcInner's count.
            # Its two Options and entry pointer occupy 40 bytes; color follows.
            raw = read(address, 41)
            left_tag, right_tag, color = raw[0], raw[16], raw[40]
            assert left_tag in (0, 1) and right_tag in (0, 1) and color in (0, 1), (left_tag, right_tag, color)
            left = u64(address + 8) if left_tag else None
            right = u64(address + 24) if right_tag else None
            entry = u64(address + 32)
            key_cap, key, key_len, value_cap, value, value_len = struct.unpack("<6Q", read(entry, 48))
            assert key_len == 27 and key_len <= key_cap <= 1024
            assert value_len == 128 and value_len <= value_cap <= 1024
            nodes.append({"node": address, "path": path, "color": color,
                          "node_count": u64(address - 8), "left": left, "right": right,
                          "entry": entry, "entry_count": u64(entry - 8),
                          "key": key, "key_cap": key_cap, "key_hex": read(key, key_len).hex(),
                          "value": value, "value_cap": value_cap, "value_hex": read(value, value_len).hex()})
            if right is not None:
                pending.append((right, path + "R"))
            if left is not None:
                pending.append((left, path + "L"))
        assert len(nodes) == 4096
        inspection = json.loads(Path(os.environ["KV9_LAYOUT_INSPECTION"]).read_text())
        function = inspection["functions"]["answer"]
        load_base = reg("rip") - function["start"]
        memcmp = u64(load_base + function["get_memcmp"]["slot"])
        maps = Path(f"/proc/{inferior.pid}/maps").read_text()
        mapping = None
        for line in maps.splitlines():
            parts = line.split()
            first, last = (int(s, 16) for s in parts[0].split("-"))
            if first <= memcmp < last:
                mapping = {"path": parts[-1], "file_offset": int(parts[2], 16) + memcmp - first,
                           "permissions": parts[1]}
        assert mapping and "libc.so" in mapping["path"]
        mapping["library_sha256"] = hashlib.sha256(Path(mapping["path"]).read_bytes()).hexdigest()
        mapping["entry_bytes_hex"] = read(memcmp, 64).hex()
        result = {"complete": True, "pid": inferior.pid, "root": root, "map": map_address,
                  "query_hex": query.hex(), "operation": operation, "nodes": nodes,
                  "runtime_memcmp": mapping, "answer_address": reg("rip"), "load_base": load_base,
                  "binary_sha256": inspection["sha256"], "performance_evidence": False,
                  "scope": "First map only; private layout decoded from exact ELF and checked independently. Debugger timing excluded."}
        destination = Path(os.environ["KV9_LAYOUT_CAPTURE"])
        with destination.open("x") as out:
            json.dump(result, out, indent=2)
            out.write("\n")
        capture_finished = True
        return False


breakpoint = Capture("*" + os.environ["KV9_LAYOUT_SYMBOL"])
breakpoint.silent = True
