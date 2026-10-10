//! Bounded static PE inspection; never executes inspected code.
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub struct Pe {
    pub machine: u16,
    pub image_base: u64,
    pub entry_rva: u32,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
}

fn slice(b: &[u8], pos: usize, len: usize) -> Result<&[u8], String> {
    let end = pos.checked_add(len).ok_or("PE offset overflow")?;
    b.get(pos..end)
        .ok_or_else(|| format!("truncated PE at 0x{pos:x}"))
}
fn u16_at(b: &[u8], pos: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(slice(b, pos, 2)?.try_into().unwrap()))
}
fn u32_at(b: &[u8], pos: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(slice(b, pos, 4)?.try_into().unwrap()))
}
fn cstring(b: &[u8], pos: usize) -> Result<String, String> {
    let rest = slice(b, pos, b.len().checked_sub(pos).ok_or("bad string offset")?)?;
    let end = rest
        .iter()
        .position(|&c| c == 0)
        .ok_or("unterminated PE string")?;
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}

impl Pe {
    pub fn read(path: &Path) -> Result<Self, String> {
        Self::parse(&fs::read(path).map_err(|e| e.to_string())?)
    }

    pub fn parse(b: &[u8]) -> Result<Self, String> {
        if slice(b, 0, 2)? != b"MZ" {
            return Err("missing MZ signature".into());
        }
        let pe = u32_at(b, 0x3c)? as usize;
        if slice(b, pe, 4)? != b"PE\0\0" {
            return Err("missing PE signature".into());
        }
        let machine = u16_at(b, pe + 4)?;
        let count = u16_at(b, pe + 6)? as usize;
        let optional = pe + 24;
        let optional_len = u16_at(b, pe + 20)? as usize;
        slice(b, optional, optional_len)?;
        let (base_offset, dir_offset, num_offset) = match u16_at(b, optional)? {
            0x20b => (24, 112, 108),
            0x10b => (28, 96, 92),
            _ => return Err("unsupported PE optional header".into()),
        };
        if optional_len < dir_offset {
            return Err("short optional header".into());
        }
        let image_base = if dir_offset == 112 {
            u64::from_le_bytes(slice(b, optional + base_offset, 8)?.try_into().unwrap())
        } else {
            u32_at(b, optional + base_offset)? as u64
        };
        let entry_rva = u32_at(b, optional + 16)?;
        let headers_size = u32_at(b, optional + 60)?;
        let sections = optional + optional_len;
        slice(
            b,
            sections,
            count.checked_mul(40).ok_or("too many sections")?,
        )?;
        let rva_offset = |rva: u32, len: usize| -> Result<usize, String> {
            if rva < headers_size
                && (rva as usize)
                    .checked_add(len)
                    .is_some_and(|n| n <= headers_size as usize)
            {
                slice(b, rva as usize, len)?;
                return Ok(rva as usize);
            }
            for i in 0..count {
                let s = sections + i * 40;
                let start = u32_at(b, s + 12)?;
                let raw_len = u32_at(b, s + 16)?;
                let raw = u32_at(b, s + 20)?;
                if let Some(delta) = rva.checked_sub(start)
                    && (delta as usize)
                        .checked_add(len)
                        .is_some_and(|n| n <= raw_len as usize)
                {
                    let pos = (raw as usize)
                        .checked_add(delta as usize)
                        .ok_or("bad section offset")?;
                    slice(b, pos, len)?;
                    return Ok(pos);
                }
            }
            Err(format!("unmapped RVA 0x{rva:x}"))
        };
        let dirs = u32_at(b, optional + num_offset)? as usize;
        let directory = |index: usize| -> Result<(u32, u32), String> {
            if index >= dirs {
                return Ok((0, 0));
            }
            let off = dir_offset + index * 8;
            if off + 8 > optional_len {
                return Err("truncated data directories".into());
            }
            Ok((u32_at(b, optional + off)?, u32_at(b, optional + off + 4)?))
        };
        let (imports_rva, imports_len) = directory(1)?;
        let mut imports = Vec::new();
        if imports_rva != 0 {
            let mut terminated = false;
            for i in 0..(imports_len as usize / 20).min(b.len() / 20) {
                let rva = imports_rva
                    .checked_add((i * 20) as u32)
                    .ok_or("import RVA overflow")?;
                let pos = rva_offset(rva, 20)?;
                if slice(b, pos, 20)?.iter().all(|&c| c == 0) {
                    terminated = true;
                    break;
                }
                let name_rva = u32_at(b, pos + 12)?;
                imports.push(cstring(b, rva_offset(name_rva, 1)?)?);
            }
            if !terminated {
                return Err("unterminated import descriptors".into());
            }
        }
        let (export_rva, _) = directory(0)?;
        let mut exports = Vec::new();
        if export_rva != 0 {
            let pos = rva_offset(export_rva, 40)?;
            let names_count = u32_at(b, pos + 24)? as usize;
            if names_count > b.len() / 4 {
                return Err("too many export names".into());
            }
            let names_rva = u32_at(b, pos + 32)?;
            if names_count != 0 {
                let names = rva_offset(names_rva, names_count * 4)?;
                for i in 0..names_count {
                    exports.push(cstring(b, rva_offset(u32_at(b, names + i * 4)?, 1)?)?);
                }
            }
        }
        Ok(Self {
            machine,
            image_base,
            entry_rva,
            imports,
            exports,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_images_fail_without_panicking() {
        for b in [&[][..], b"MZ", &[0; 128], &[255; 512]] {
            assert!(Pe::parse(b).is_err());
        }
        let mut b = vec![0; 128];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Pe::parse(&b).is_err());
    }
    #[test]
    #[cfg(windows)]
    fn reads_real_system_image() {
        let root = std::env::var_os("SystemRoot").unwrap();
        let pe = Pe::read(&Path::new(&root).join("System32/kernel32.dll")).unwrap();
        assert_eq!(pe.machine, 0x8664);
        assert!(pe.exports.iter().any(|s| s == "GetProcAddress"));
    }
}
