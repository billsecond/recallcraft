//! Opus in an Ogg file (`.opus`, RFC 7845): what every current player and browser opens.

use std::io::Write;

use crate::opus::FRAME;

/// The Ogg checksum (CRC-32, polynomial 0x04C11DB7, not reflected).
const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut r = (i as u32) << 24;
        let mut j = 0;
        while j < 8 {
            r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
            j += 1;
        }
        t[i] = r;
        i += 1;
    }
    t
}

const CRC: [u32; 256] = crc_table();

fn crc(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |c, b| (c << 8) ^ CRC.get((((c >> 24) as u8) ^ b) as usize).copied().unwrap_or(0))
}

/// The encoder's warm-up a player skips (at 48 kHz): rusty-opus's lookahead.
pub const PRE_SKIP: u16 = 120;

/// An Ogg Opus stream being written.
pub struct OggOpus<W: Write> {
    out: W,
    serial: u32,
    sequence: u32,
    granule: u64,
    page: Vec<Vec<u8>>,
}

impl<W: Write> OggOpus<W> {
    /// Writes the two header pages for a mono stream at 48 kHz.
    pub fn new(out: W, serial: u32) -> std::io::Result<OggOpus<W>> {
        let mut o = OggOpus { out, serial, sequence: 0, granule: 0, page: Vec::new() };
        let mut head = Vec::new();
        head.extend_from_slice(b"OpusHead");
        head.push(1);
        head.push(1);
        head.extend_from_slice(&PRE_SKIP.to_le_bytes());
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        o.write_page(&[head], 0, 0x02)?;
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        let vendor = b"RecallCraft";
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        o.write_page(&[tags], 0, 0)?;
        o.granule = u64::from(PRE_SKIP);
        Ok(o)
    }

    fn write_page(&mut self, packets: &[Vec<u8>], granule: u64, flags: u8) -> std::io::Result<()> {
        let mut lacing = Vec::new();
        for p in packets {
            let mut n = p.len();
            while n >= 255 {
                lacing.push(255u8);
                n -= 255;
            }
            lacing.push(n as u8);
        }
        let mut page = Vec::with_capacity(27 + lacing.len() + packets.iter().map(Vec::len).sum::<usize>());
        page.extend_from_slice(b"OggS");
        page.push(0);
        page.push(flags);
        page.extend_from_slice(&granule.to_le_bytes());
        page.extend_from_slice(&self.serial.to_le_bytes());
        page.extend_from_slice(&self.sequence.to_le_bytes());
        page.extend_from_slice(&0u32.to_le_bytes());
        page.push(lacing.len() as u8);
        page.extend_from_slice(&lacing);
        for p in packets {
            page.extend_from_slice(p);
        }
        let sum = crc(&page);
        if let Some(c) = page.get_mut(22..26) {
            c.copy_from_slice(&sum.to_le_bytes());
        }
        self.sequence += 1;
        self.out.write_all(&page)
    }

    /// Adds one 20 ms packet.
    pub fn packet(&mut self, p: Vec<u8>) -> std::io::Result<()> {
        self.page.push(p);
        self.granule += FRAME as u64;
        // A page a second: 50 packets, at most 255 lacing values.
        if self.page.len() >= 50 {
            let page = std::mem::take(&mut self.page);
            self.write_page(&page, self.granule, 0)?;
        }
        Ok(())
    }

    /// The last page, marked as the end.
    pub fn finish(mut self) -> std::io::Result<W> {
        let page = std::mem::take(&mut self.page);
        self.write_page(&page, self.granule, 0x04)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksum_is_ogg_s() {
        // The CRC of "OggS" with this polynomial, as the Ogg reference computes it.
        assert_eq!(crc(b""), 0);
        let mut page = b"OggS\0\x02\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\x01\x13OpusHead\x01\x01\x38\x01\x80\xbb\0\0\0\0\0".to_vec();
        let sum = crc(&page);
        page[22..26].copy_from_slice(&sum.to_le_bytes());
        // Checking a page with its own sum in place gives the same sum computed over zeros.
        let mut zeroed = page.clone();
        zeroed[22..26].copy_from_slice(&[0; 4]);
        assert_eq!(crc(&zeroed), sum);
    }

    #[test]
    fn pages_are_laced_and_ended() {
        let mut o = OggOpus::new(Vec::new(), 7).unwrap();
        for i in 0..120 {
            o.packet(vec![i as u8; if i == 3 { 600 } else { 40 }]).unwrap();
        }
        let bytes = o.finish().unwrap();
        let pages: Vec<usize> = bytes.windows(4).enumerate().filter(|(_, w)| *w == b"OggS").map(|(i, _)| i).collect();
        assert_eq!(pages.len(), 2 + 3, "two headers, two full pages and the last");
        let last = *pages.last().unwrap();
        assert_eq!(bytes[last + 5], 0x04, "the end is marked");
        let granule = u64::from_le_bytes(bytes[last + 6..last + 14].try_into().unwrap());
        assert_eq!(granule, u64::from(PRE_SKIP) + 120 * FRAME as u64);
    }
}
