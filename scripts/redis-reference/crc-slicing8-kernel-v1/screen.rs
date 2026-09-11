use std::hint::black_box;
use std::time::Instant;
mod old {
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    crc32_parts(&[bytes])
}
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut byte = 0;
    while byte < table.len() {
        let mut crc = byte as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            bit += 1;
        }
        table[byte] = crc;
        byte += 1;
    }
    table
};
pub(crate) fn crc32_parts(parts: &[&[u8]]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for part in parts {
        for &byte in *part {
            crc = (crc >> 8) ^ CRC32_TABLE[((crc ^ u32::from(byte)) & 0xff) as usize];
        }
    }
    !crc
}
}
mod new {
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    crc32_parts(&[bytes])
}
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut byte = 0;
    while byte < table.len() {
        let mut crc = byte as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            bit += 1;
        }
        table[byte] = crc;
        byte += 1;
    }
    table
};
const CRC32_SLICING: [[u32; 256]; 8] = {
    let mut tables = [[0; 256]; 8];
    tables[0] = CRC32_TABLE;
    let mut slice = 1;
    while slice < tables.len() {
        let mut byte = 0;
        while byte < 256 {
            let prior = tables[slice - 1][byte];
            tables[slice][byte] = (prior >> 8) ^ CRC32_TABLE[(prior & 0xff) as usize];
            byte += 1;
        }
        slice += 1;
    }
    tables
};
pub(crate) fn crc32_parts(parts: &[&[u8]]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for part in parts {
        let mut chunks = part.chunks_exact(8);
        for chunk in &mut chunks {
            let low = crc ^ u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            crc = CRC32_SLICING[7][(low & 0xff) as usize]
                ^ CRC32_SLICING[6][((low >> 8) & 0xff) as usize]
                ^ CRC32_SLICING[5][((low >> 16) & 0xff) as usize]
                ^ CRC32_SLICING[4][(low >> 24) as usize]
                ^ CRC32_SLICING[3][usize::from(chunk[4])]
                ^ CRC32_SLICING[2][usize::from(chunk[5])]
                ^ CRC32_SLICING[1][usize::from(chunk[6])]
                ^ CRC32_SLICING[0][usize::from(chunk[7])];
        }
        for &byte in chunks.remainder() {
            crc = (crc >> 8) ^ CRC32_TABLE[((crc ^ u32::from(byte)) & 0xff) as usize];
        }
    }
    !crc
}
}

fn main() {
    let mut state = 0x759ab31du32;
    let bytes: Vec<u8> = (0..16400).map(|_| { state ^= state << 13; state ^= state >> 17; state ^= state << 5; state as u8 }).collect();
    println!("repeat,length,role,calls,elapsed_ns,checksum");
    for repeat in 0..4 {
        for length in [1usize, 7, 8, 16, 28, 64, 128, 8192, 16384] {
            let data = &bytes[3..3+length];
            assert_eq!(old::crc32(data), new::crc32(data));
            let calls = (16_777_216usize / length).clamp(4096, 200_000);
            let roles = if repeat % 2 == 0 { ["old", "new"] } else { ["new", "old"] };
            for role in roles {
                let checksum: fn(&[u8]) -> u32 = if role == "old" { old::crc32 } else { new::crc32 };
                for _ in 0..128 { black_box(checksum(black_box(data))); }
                let started = Instant::now();
                let mut result = 0u32;
                for _ in 0..calls { result = result.wrapping_add(black_box(checksum(black_box(data)))); }
                let elapsed = started.elapsed().as_nanos();
                println!("{repeat},{length},{role},{calls},{elapsed},{result}");
            }
        }
    }
}
