//! Self-contained MDict (.mdx/.mdd) parser.
//! Port of the reference Python `readmdict` logic, but independent.
//! Supports zlib (engine >= 2.0) and no-compression blocks. LZO v1 files
//! are rejected with a clear error (we don't want a native LZO dep in core).

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use flate2::read::ZlibDecoder;
use ripemd::Ripemd128;
use ripemd::Digest;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad mdx: {0}")]
    Bad(String),
    #[error("unsupported compression: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn bad<T>(m: impl Into<String>) -> Result<T> {
    Err(Error::Bad(m.into()))
}

/// One decoded (key, record) pair.
pub type Item = (Vec<u8>, Vec<u8>);

pub struct Mdx {
    path: String,
    encoding: String,
    header: HashMap<String, String>,
    version: f64,
    number_width: usize,
    encrypt: u32,
    num_entries: usize,
    key_list: Vec<(u64, Vec<u8>)>,
    record_block_offset: u64,
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

impl Mdx {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut f = std::fs::File::open(&path)?;

        // ---- header text ----
        let mut hsize_buf = [0u8; 4];
        f.read_exact(&mut hsize_buf)?;
        let header_bytes_size = u32::from_be_bytes(hsize_buf) as usize;
        let mut header_bytes = vec![0u8; header_bytes_size];
        f.read_exact(&mut header_bytes)?;

        let mut adler_buf = [0u8; 4];
        f.read_exact(&mut adler_buf)?;
        let expected = u32::from_le_bytes(adler_buf);
        if expected != adler32(&header_bytes) {
            return bad("header adler32 mismatch");
        }
        let key_block_offset = f.stream_position()?;

        // header text is UTF-16, ends with 0x00 0x00
        let header_text = String::from_utf16(
            &header_bytes[..header_bytes.len().saturating_sub(2)]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<u16>>(),
        )
        .unwrap_or_default();

        let header = parse_header(&header_text);
        let encoding = header
            .get("Encoding")
            .cloned()
            .unwrap_or_else(|| "UTF-8".to_string());
        let encoding = match encoding.as_str() {
            "GBK" | "GB2312" => "GB18030".to_string(),
            s => s.to_string(),
        };
        let encrypt = match header.get("Encrypted").map(|s| s.as_str()) {
            None | Some("No") => 0,
            Some("Yes") => 1,
            Some(s) => s.parse().unwrap_or(0),
        };
        let version: f64 = header
            .get("GeneratedByEngineVersion")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0);
        let number_width = if version < 2.0 { 4 } else { 8 };

        let mut m = Mdx {
            path: path.display().to_string(),
            encoding,
            header,
            version,
            number_width,
            encrypt,
            num_entries: 0,
            key_list: Vec::new(),
            record_block_offset: 0,
        };

        let key_list = m.read_keys_at(key_block_offset).or_else(|_| {
            // brutal force fallback (encrypted / offset variance)
            m.read_keys_brutal(key_block_offset)
        })?;
        m.num_entries = key_list.len();
        m.key_list = key_list;
        Ok(m)
    }

    pub fn len(&self) -> usize {
        self.num_entries
    }

    pub fn header(&self) -> &HashMap<String, String> {
        &self.header
    }

    pub fn items(&self) -> Vec<Item> {
        self.decode_record_block().unwrap_or_default()
    }

    fn read_number<R: Read>(&self, r: &mut R) -> Result<u64> {
        let mut buf = vec![0u8; self.number_width];
        r.read_exact(&mut buf)?;
        Ok(if self.number_width == 4 {
            u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as u64
        } else {
            u64::from_be_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ])
        })
    }

    fn read_keys_at(&mut self, offset: u64) -> Result<Vec<(u64, Vec<u8>)>> {
        let mut f = std::fs::File::open(&self.path)?;
        f.seek(SeekFrom::Start(offset))?;

        let num_bytes = if self.version >= 2.0 { 8 * 5 } else { 4 * 4 };
        let mut block = vec![0u8; num_bytes];
        f.read_exact(&mut block)?;

        let mut cur = std::io::Cursor::new(&block);
        let num_key_blocks = self.read_number(&mut cur)?;
        self.num_entries = self.read_number(&mut cur)? as usize;
        let _info_decomp = if self.version >= 2.0 {
            self.read_number(&mut cur)?
        } else {
            0
        };
        let info_size = self.read_number(&mut cur)? as usize;
        let key_block_size = self.read_number(&mut cur)? as usize;

        if self.version >= 2.0 {
            let mut a = [0u8; 4];
            f.read_exact(&mut a)?;
            if u32::from_be_bytes(a) != adler32(&block) {
                return bad("key numbers adler32 mismatch");
            }
        }

        let mut key_block_info = vec![0u8; info_size];
        f.read_exact(&mut key_block_info)?;
        let info_list = self.decode_key_block_info(&key_block_info)?;
        if num_key_blocks as usize != info_list.len() {
            return bad("key block count mismatch");
        }

        let mut key_block_compressed = vec![0u8; key_block_size];
        f.read_exact(&mut key_block_compressed)?;
        let key_list = self.decode_key_block(&key_block_compressed, &info_list)?;
        self.record_block_offset = f.stream_position()?;
        Ok(key_list)
    }

    fn read_keys_brutal(&mut self, offset: u64) -> Result<Vec<(u64, Vec<u8>)>> {
        let mut f = std::fs::File::open(&self.path)?;
        f.seek(SeekFrom::Start(offset))?;
        let (num_bytes, marker): (u64, &[u8]) = if self.version >= 2.0 {
            (8 * 5 + 4, b"\x02\x00\x00\x00")
        } else {
            (4 * 4, b"\x01\x00\x00\x00")
        };
        let mut _block = vec![0u8; num_bytes as usize];
        f.read_exact(&mut _block)?;

        let mut key_block_info = vec![0u8; 8];
        f.read_exact(&mut key_block_info)?;
        if self.version >= 2.0 && &key_block_info[..4] != b"\x02\x00\x00\x00" {
            return bad("invalid key block marker");
        }
        // read until marker
        loop {
            let fpos = f.stream_position()?;
            let mut buf = [0u8; 1024];
            let n = f.read(&mut buf)?;
            if n == 0 {
                return bad("key block marker not found");
            }
            let buf = &buf[..n];
            if let Some(idx) = find_sub(buf, marker) {
                key_block_info.extend_from_slice(&buf[..idx]);
                f.seek(SeekFrom::Start(fpos + idx as u64))?;
                break;
            } else {
                key_block_info.extend_from_slice(buf);
            }
        }
        let info_list = self.decode_key_block_info(&key_block_info)?;
        let key_block_size: usize = info_list.iter().map(|(s, _)| *s as usize).sum();
        let mut kbc = vec![0u8; key_block_size];
        f.read_exact(&mut kbc)?;
        let key_list = self.decode_key_block(&kbc, &info_list)?;
        self.record_block_offset = f.stream_position()?;
        self.num_entries = key_list.len();
        Ok(key_list)
    }

    fn decode_key_block_info(&self, data: &[u8]) -> Result<Vec<(u64, u64)>> {
        // decrypt key info block if needed (before decompress)
        let data: &[u8] = if self.encrypt & 0x02 != 0 {
            &mdx_decrypt(data)
        } else {
            data
        };
        let key_block_info: Vec<u8> = if self.version >= 2.0 {
            if &data[..4] != b"\x02\x00\x00\x00" {
                return bad("key block info not zlib");
            }
            let expected = be_u32(&data[4..8]);
            let mut d = ZlibDecoder::new(&data[8..]);
            let mut out = Vec::new();
            d.read_to_end(&mut out)?;
            if expected != adler32(&out) {
                return bad("key block info adler32 mismatch");
            }
            out
        } else {
            data.to_vec()
        };

        let (bw, term) = if self.version >= 2.0 { (2usize, 1usize) } else { (1, 0) };
        let mut list = Vec::new();
        let mut i = 0usize;
        while i < key_block_info.len() {
            i += self.number_width; // num entries in block (unused)
            // text head
            let hsize = read_u16(&key_block_info[i..i + bw]) as usize;
            i += bw;
            i += if self.encoding != "UTF-16" { hsize + term } else { (hsize + term) * 2 };
            // text tail
            let tsize = read_u16(&key_block_info[i..i + bw]) as usize;
            i += bw;
            i += if self.encoding != "UTF-16" { tsize + term } else { (tsize + term) * 2 };
            let cs = self.read_number_from_bytes(&key_block_info[i..])?;
            i += self.number_width;
            let ds = self.read_number_from_bytes(&key_block_info[i..])?;
            i += self.number_width;
            list.push((cs, ds));
        }
        Ok(list)
    }

    fn decode_key_block(&self, data: &[u8], info_list: &[(u64, u64)]) -> Result<Vec<(u64, Vec<u8>)>> {
        let mut key_list = Vec::new();
        let mut i = 0usize;
        for &(cs, ds) in info_list {
            let start = i;
            let end = i + cs as usize;
            if end > data.len() {
                break;
            }
            let block_type = &data[start..start + 4];
            let expected = be_u32(&data[start + 4..start + 8]);
            let key_block: Vec<u8> = if block_type == b"\x00\x00\x00\x00" {
                data[start + 8..end].to_vec()
            } else if block_type == b"\x01\x00\x00\x00" {
                return Err(Error::Unsupported("LZO compression".into()));
            } else if block_type == b"\x02\x00\x00\x00" {
                let mut d = ZlibDecoder::new(&data[start + 8..end]);
                let mut out = Vec::with_capacity(ds as usize);
                d.read_to_end(&mut out)?;
                out
            } else {
                return bad("unknown key block compression");
            };
            if expected != adler32(&key_block) {
                return bad("key block adler32 mismatch");
            }
            key_list.extend(self.split_key_block(&key_block)?);
            i += cs as usize;
        }
        Ok(key_list)
    }

    fn split_key_block(&self, block: &[u8]) -> Result<Vec<(u64, Vec<u8>)>> {
        let mut out = Vec::new();
        let (delim, width) = if self.encoding == "UTF-16" {
            ([0u8, 0u8].as_slice(), 2usize)
        } else {
            ([0u8].as_slice(), 1usize)
        };
        let mut idx = 0usize;
        while idx < block.len() {
            if idx + self.number_width > block.len() {
                break;
            }
            let key_id = self.read_number_from_bytes(&block[idx..])?;
            let mut i = idx + self.number_width;
            let mut end = block.len();
            while i + width <= block.len() {
                if &block[i..i + width] == delim {
                    end = i;
                    break;
                }
                i += width;
            }
            let text = decode_text(&block[idx + self.number_width..end], &self.encoding);
            out.push((key_id, text));
            idx = end + width;
        }
        Ok(out)
    }

    fn decode_record_block(&self) -> Result<Vec<Item>> {
        let mut f = std::fs::File::open(&self.path)?;
        f.seek(SeekFrom::Start(self.record_block_offset))?;

        let num_blocks = self.read_number(&mut f)?;
        let num_entries = self.read_number(&mut f)?;
        let info_size = self.read_number(&mut f)? as usize;
        let block_size = self.read_number(&mut f)? as usize;

        let mut info_list = Vec::new();
        for _ in 0..num_blocks {
            let cs = self.read_number(&mut f)?;
            let ds = self.read_number(&mut f)?;
            info_list.push((cs, ds));
        }

        let mut items = Vec::new();
        let mut offset = 0usize;
        let mut ki = 0usize;
        let mut size_counter = 0usize;
        for (cs, ds) in info_list {
            let mut comp = vec![0u8; cs as usize];
            f.read_exact(&mut comp)?;
            let block_type = &comp[..4];
            let expected = be_u32(&comp[4..8]);
            let block: Vec<u8> = if block_type == b"\x00\x00\x00\x00" {
                comp[8..].to_vec()
            } else if block_type == b"\x01\x00\x00\x00" {
                return Err(Error::Unsupported("LZO compression".into()));
            } else if block_type == b"\x02\x00\x00\x00" {
                let mut d = ZlibDecoder::new(&comp[8..]);
                let mut out = Vec::with_capacity(ds as usize);
                d.read_to_end(&mut out)?;
                out
            } else {
                return bad("unknown record block compression");
            };
            if expected != adler32(&block) {
                return bad("record block adler32 mismatch");
            }
            while ki < self.key_list.len() {
                let (record_start, key_text) = &self.key_list[ki];
                let rs = *record_start as usize;
                if rs - offset >= block.len() {
                    break;
                }
                let record_end = if ki < self.key_list.len() - 1 {
                    self.key_list[ki + 1].0 as usize
                } else {
                    block.len() + offset
                };
                ki += 1;
                let rec = &block[rs - offset..record_end - offset];
                let text = decode_text(rec, &self.encoding);
                items.push((key_text.clone(), text));
            }
            offset += block.len();
            size_counter += cs as usize;
        }
        let _ = (num_entries, info_size, block_size, size_counter);
        Ok(items)
    }

    fn read_number_from_bytes(&self, b: &[u8]) -> Result<u64> {
        if self.number_width == 4 {
            if b.len() < 4 {
                return bad("short number");
            }
            Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64)
        } else {
            if b.len() < 8 {
                return bad("short number");
            }
            Ok(u64::from_be_bytes([
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
            ]))
        }
    }
}

fn read_u16(b: &[u8]) -> u16 {
    if b.len() < 2 {
        return 0;
    }
    u16::from_be_bytes([b[0], b[1]])
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

fn decode_text(data: &[u8], encoding: &str) -> Vec<u8> {
    match encoding {
        "UTF-16" | "UTF-16LE" => {
            let mut trimmed = data.to_vec();
            while trimmed.last() == Some(&0) {
                trimmed.pop();
            }
            let units: Vec<u16> = trimmed
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units).into_bytes()
        }
        _ => {
            // best-effort: treat as UTF-8 (most modern dicts); GB18030 needs a
            // proper codec we intentionally avoid in core. Pass through raw otherwise.
            String::from_utf8_lossy(data).into_owned().into_bytes()
        }
    }
}

/// fast_decrypt: port of Python `_fast_decrypt`.
fn fast_decrypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut prev = 0x36u8;
    for (i, &b) in data.iter().enumerate() {
        let t = ((b >> 4) | (b << 4)) & 0xff;
        let t = t ^ prev ^ (i as u8) ^ key[i % key.len()];
        prev = b;
        out.push(t);
    }
    out
}

/// mdx_decrypt: port of Python `_mdx_decrypt` using RIPEMD-128.
fn mdx_decrypt(comp_block: &[u8]) -> Vec<u8> {
    // key = ripemd128(comp_block[4:8] + pack('<L', 0x3695))
    let mut seed = comp_block[4..8].to_vec();
    seed.extend_from_slice(&0x3695u32.to_le_bytes());
    let key = ripemd128(&seed);
    let mut out = comp_block[0..8].to_vec();
    out.extend(fast_decrypt(&comp_block[8..], &key));
    out
}

fn ripemd128(data: &[u8]) -> Vec<u8> {
    let mut h = Ripemd128::new();
    h.update(data);
    h.finalize().to_vec()
}


fn parse_header(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let re = regex::Regex::new(r#"(\w+)="(.*?)""#).unwrap();
    for cap in re.captures_iter(text) {
        let k = unescape(&cap[1]);
        let v = unescape(&cap[2]);
        map.insert(k, v);
    }
    map
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}
