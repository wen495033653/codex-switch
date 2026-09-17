use super::codex_home::{ensure_session_relative_path, normalize_relative_path};
use std::io::Seek;
use std::{collections::HashMap, fs, io::Write, path::Path};

const ZIP_LOCAL_FILE_HEADER: u32 = 0x0403_4b50;

const ZIP_CENTRAL_DIRECTORY_HEADER: u32 = 0x0201_4b50;

const ZIP_END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;

const ZIP_UTF8_FLAG: u16 = 1 << 11;

struct ZipCentralEntry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

#[derive(Debug, Clone)]
pub(super) struct ZipArchiveLite {
    pub(super) data: Vec<u8>,
    pub(super) entries: HashMap<String, ZipReadEntry>,
}

#[derive(Debug, Clone)]
pub(super) struct ZipReadEntry {
    method: u16,
    crc: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u32,
}

impl ZipArchiveLite {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        let data =
            fs::read(path).map_err(|err| format!("读取导入 zip 失败 {}: {err}", path.display()))?;
        Self::from_bytes(data)
    }

    fn from_bytes(data: Vec<u8>) -> Result<Self, String> {
        let eocd = find_eocd(&data).ok_or_else(|| "未找到 zip central directory".to_string())?;
        let disk = read_u16_at(&data, eocd + 4)?;
        let central_disk = read_u16_at(&data, eocd + 6)?;
        if disk != 0 || central_disk != 0 {
            return Err("不支持分卷 zip".to_string());
        }
        let entry_count = read_u16_at(&data, eocd + 10)? as usize;
        let central_size = read_u32_at(&data, eocd + 12)? as usize;
        let central_offset = read_u32_at(&data, eocd + 16)? as usize;
        if central_offset + central_size > data.len() {
            return Err("zip central directory 越界".to_string());
        }

        let mut entries = HashMap::new();
        let mut cursor = central_offset;
        for _ in 0..entry_count {
            if read_u32_at(&data, cursor)? != ZIP_CENTRAL_DIRECTORY_HEADER {
                return Err("zip central directory 结构无效".to_string());
            }
            let flags = read_u16_at(&data, cursor + 8)?;
            let method = read_u16_at(&data, cursor + 10)?;
            let crc = read_u32_at(&data, cursor + 16)?;
            let compressed_size = read_u32_at(&data, cursor + 20)?;
            let uncompressed_size = read_u32_at(&data, cursor + 24)?;
            let name_len = read_u16_at(&data, cursor + 28)? as usize;
            let extra_len = read_u16_at(&data, cursor + 30)? as usize;
            let comment_len = read_u16_at(&data, cursor + 32)? as usize;
            let local_header_offset = read_u32_at(&data, cursor + 42)?;
            let name_start = cursor + 46;
            let name_end = name_start + name_len;
            if name_end > data.len() {
                return Err("zip 条目名越界".to_string());
            }
            let name = if flags & ZIP_UTF8_FLAG != 0 {
                String::from_utf8(data[name_start..name_end].to_vec())
                    .map_err(|_| "zip 条目名不是 UTF-8".to_string())?
            } else {
                String::from_utf8_lossy(&data[name_start..name_end]).to_string()
            };
            if name != "manifest.json" {
                let relative = normalize_relative_path(&name)?;
                ensure_session_relative_path(&relative)?;
            }
            entries.insert(
                name,
                ZipReadEntry {
                    method,
                    crc,
                    compressed_size,
                    uncompressed_size,
                    local_header_offset,
                },
            );
            cursor = name_end + extra_len + comment_len;
            if cursor > data.len() {
                return Err("zip central directory 条目越界".to_string());
            }
        }
        Ok(Self { data, entries })
    }

    pub(super) fn read_entry(&self, name: &str) -> Result<Vec<u8>, String> {
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| format!("zip 中缺少文件: {name}"))?;
        if entry.method != 0 {
            return Err(format!("zip 文件 {name} 使用了不支持的压缩方式"));
        }
        if entry.compressed_size != entry.uncompressed_size {
            return Err(format!("zip 文件 {name} 大小信息不一致"));
        }
        let offset = entry.local_header_offset as usize;
        if read_u32_at(&self.data, offset)? != ZIP_LOCAL_FILE_HEADER {
            return Err(format!("zip 文件 {name} 的本地头无效"));
        }
        let name_len = read_u16_at(&self.data, offset + 26)? as usize;
        let extra_len = read_u16_at(&self.data, offset + 28)? as usize;
        let start = offset + 30 + name_len + extra_len;
        let end = start + entry.uncompressed_size as usize;
        if end > self.data.len() {
            return Err(format!("zip 文件 {name} 数据越界"));
        }
        let data = self.data[start..end].to_vec();
        if crc32(&data) != entry.crc {
            return Err(format!("zip 文件 {name} CRC 校验失败"));
        }
        Ok(data)
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

pub(super) fn write_zip_store(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("创建导出目录失败 {}: {err}", parent.display()))?;
    }
    let mut file = fs::File::create(path)
        .map_err(|err| format!("创建 zip 文件失败 {}: {err}", path.display()))?;
    let mut central_entries = Vec::new();
    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        if name_bytes.len() > u16::MAX as usize {
            return Err(format!("zip 条目路径过长: {name}"));
        }
        if data.len() > u32::MAX as usize {
            return Err(format!("zip 条目过大: {name}"));
        }
        let offset = file
            .stream_position()
            .map_err(|err| format!("读取 zip 写入位置失败: {err}"))?;
        if offset > u32::MAX as u64 {
            return Err("zip 文件过大，V1 不支持 Zip64".to_string());
        }
        let crc = crc32(data);
        write_u32(&mut file, ZIP_LOCAL_FILE_HEADER)?;
        write_u16(&mut file, 20)?;
        write_u16(&mut file, ZIP_UTF8_FLAG)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 33)?;
        write_u32(&mut file, crc)?;
        write_u32(&mut file, data.len() as u32)?;
        write_u32(&mut file, data.len() as u32)?;
        write_u16(&mut file, name_bytes.len() as u16)?;
        write_u16(&mut file, 0)?;
        file.write_all(name_bytes)
            .map_err(|err| format!("写入 zip 条目名失败: {err}"))?;
        file.write_all(data)
            .map_err(|err| format!("写入 zip 条目失败: {err}"))?;
        central_entries.push(ZipCentralEntry {
            name: name.clone(),
            crc,
            size: data.len() as u32,
            offset: offset as u32,
        });
    }
    let central_start = file
        .stream_position()
        .map_err(|err| format!("读取 zip central directory 位置失败: {err}"))?;
    if central_start > u32::MAX as u64 {
        return Err("zip 文件过大，V1 不支持 Zip64".to_string());
    }
    for entry in &central_entries {
        let name_bytes = entry.name.as_bytes();
        write_u32(&mut file, ZIP_CENTRAL_DIRECTORY_HEADER)?;
        write_u16(&mut file, 20)?;
        write_u16(&mut file, 20)?;
        write_u16(&mut file, ZIP_UTF8_FLAG)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 33)?;
        write_u32(&mut file, entry.crc)?;
        write_u32(&mut file, entry.size)?;
        write_u32(&mut file, entry.size)?;
        write_u16(&mut file, name_bytes.len() as u16)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u32(&mut file, 0)?;
        write_u32(&mut file, entry.offset)?;
        file.write_all(name_bytes)
            .map_err(|err| format!("写入 zip central directory 失败: {err}"))?;
    }
    let central_end = file
        .stream_position()
        .map_err(|err| format!("读取 zip central directory 大小失败: {err}"))?;
    let central_size = central_end - central_start;
    if central_size > u32::MAX as u64 || central_entries.len() > u16::MAX as usize {
        return Err("zip 文件过大，V1 不支持 Zip64".to_string());
    }
    write_u32(&mut file, ZIP_END_OF_CENTRAL_DIRECTORY)?;
    write_u16(&mut file, 0)?;
    write_u16(&mut file, 0)?;
    write_u16(&mut file, central_entries.len() as u16)?;
    write_u16(&mut file, central_entries.len() as u16)?;
    write_u32(&mut file, central_size as u32)?;
    write_u32(&mut file, central_start as u32)?;
    write_u16(&mut file, 0)?;
    Ok(())
}

fn write_u16(file: &mut fs::File, value: u16) -> Result<(), String> {
    file.write_all(&value.to_le_bytes())
        .map_err(|err| format!("写入 zip 失败: {err}"))
}

fn write_u32(file: &mut fs::File, value: u32) -> Result<(), String> {
    file.write_all(&value.to_le_bytes())
        .map_err(|err| format!("写入 zip 失败: {err}"))
}

fn find_eocd(data: &[u8]) -> Option<usize> {
    if data.len() < 22 {
        return None;
    }
    let min = data.len().saturating_sub(65_557);
    (min..=data.len() - 22)
        .rev()
        .find(|index| read_u32_at(data, *index).ok() == Some(ZIP_END_OF_CENTRAL_DIRECTORY))
}

fn read_u16_at(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| "zip 数据越界".to_string())?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| "zip 数据越界".to_string())?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
