#!/usr/bin/env python3
"""Inspect retained x86-64 read code and resolve its static dispatch/GET targets.

This does not resolve a running process's libc IFUNC implementation or prove a
performance cause. Full disassembly and ELF identities remain in the output.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess


class Elf:
    def __init__(self, path):
        self.data = path.read_bytes()
        assert self.data[:6] == b"\x7fELF\x02\x01"
        assert struct.unpack_from("<H", self.data, 18)[0] == 62
        offset = struct.unpack_from("<Q", self.data, 32)[0]
        size, count = struct.unpack_from("<HH", self.data, 54)
        self.segments = []
        for i in range(count):
            row = struct.unpack_from("<IIQQQQQQ", self.data, offset + i * size)
            if row[0] == 1:
                self.segments.append((row[3], row[2], row[5]))

    def read(self, address, size):
        hits = [(file + address - virtual) for virtual, file, length in self.segments
                if virtual <= address and address + size <= virtual + length]
        assert len(hits) == 1
        return self.data[hits[0]:hits[0] + size]


def command(args):
    return subprocess.check_output(args, text=True, timeout=60)


def inspect(binary, output):
    output.mkdir(exist_ok=False)
    elf = Elf(binary)
    symbols = command(["nm", "-S", "-C", "--defined-only", str(binary)])
    relocations = command(["readelf", "-rW", str(binary)])
    (output / "symbols.txt").write_text(symbols)
    (output / "relocations.txt").write_text(relocations)
    result = {"binary": str(binary), "sha256": hashlib.sha256(elf.data).hexdigest(),
              "bytes": len(elf.data), "functions": {}}
    for name in ("answer", "read_panels"):
        match = re.search(r"^([0-9a-f]+) ([0-9a-f]+) t "
                          r"kv9_outlined_mutation_experiment::" + name + "$", symbols, re.M)
        assert match, name
        start, size = (int(value, 16) for value in match.groups())
        assembly = command(["objdump", "-d", "-C", "--no-show-raw-insn",
                            f"--start-address={start}", f"--stop-address={start + size}", str(binary)])
        (output / (name + ".txt")).write_text(assembly)
        shape, direct_calls = [], []
        for line in assembly.splitlines():
            instruction = re.match(r"^\s*([0-9a-f]+):\s+(.*)$", line)
            if not instruction:
                continue
            code = instruction[2].split("#")[0].strip()
            call = re.search(r"\bcall\s+([0-9a-f]+) <(.+)>", code)
            if call:
                direct_calls.append({"offset": int(instruction[1], 16) - start,
                                     "target": call[2]})
            code = re.sub(r"\b[0-9a-f]+ (?=<)", "", code)
            code = re.sub(r"-?0x[0-9a-f]+\(%rip\)", "RIP_DISPLACEMENT", code)
            shape.append(code)
        row = {"start": start, "bytes": size, "mod64": start % 64,
               "mod4096": start % 4096, "operand_shapes": shape, "direct_calls": direct_calls}
        if name == "answer":
            table = re.search(r"\blea\s+[^\n]*\(%rip\),%rax\s+# ([0-9a-f]+)", assembly)
            assert table
            address = int(table[1], 16)
            targets = [address + offset - start for offset in struct.unpack("<6i", elf.read(address, 24))]
            assert all(0 <= offset < size for offset in targets)
            row["dispatch_table"] = {"address": address, "operation_lengths": list(range(6, 12)),
                                     "relative_targets": targets}
            got = re.search(r"\bmov\s+[^\n]*\(%rip\),%rbx\s+# ([0-9a-f]+) <memcmp@", assembly)
            assert got
            slot = int(got[1], 16)
            relocation = next(line for line in relocations.splitlines()
                              if line.startswith(f"{slot:016x} "))
            assert "R_X86_64_GLOB_DAT" in relocation and "memcmp@GLIBC_2.2.5" in relocation
            row["get_memcmp"] = {"slot": slot, "relocation": relocation,
                                 "symbol": "memcmp@GLIBC_2.2.5",
                                 "runtime_ifunc_resolved": False}
        result["functions"][name] = row
    result["complete"] = True
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("binary", type=Path)
    parser.add_argument("fresh_output", type=Path)
    args = parser.parse_args()
    result = inspect(args.binary.resolve(), args.fresh_output)
    print(json.dumps({"complete": True, "sha256": result["sha256"],
                      "functions": {k: {n: v[n] for n in ("start", "bytes", "mod64", "mod4096")}
                                    for k, v in result["functions"].items()}}))


if __name__ == "__main__":
    main()
