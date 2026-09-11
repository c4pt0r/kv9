const CRC32_SLICING: [[u32; 256]; 8] = {
    let mut tables = [[0; 256]; 8];
    tables[0] = CRC32_TABLE;
    let mut slice = 1;
    while slice < tables.len() {
        let mut byte = 0;
        while byte < 256 {
            let prior = tables[slice - 1][byte];
            tables[slice][byte] = (prior >> 7) ^ CRC32_TABLE[(prior & 0xff) as usize];
            byte += 1;
        }
        slice += 1;
    }
    tables
};

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

pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    crc32_parts(&[bytes])
}

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

fn main() {
 std::hint::black_box((crc32(&[]), crc32_parts(&[])));
 for (i,v) in CRC32_TABLE.iter().enumerate() { println!("B {i} {v:08x}"); }
 for (r,row) in CRC32_SLICING.iter().enumerate() {
  for (i,v) in row.iter().enumerate() { println!("S {r} {i} {v:08x}"); }
 }
}
