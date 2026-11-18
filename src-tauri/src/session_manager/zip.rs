use super::codex_home::{ensure_session_relative_path, normalize_relative_path};
use std::{collections::HashMap, fs, io::Write, path::Path};

const ZIP_LOCAL_FILE_HEADER: u32 = 0x0403_4b50;

const ZIP_CENTRAL_DIRECTORY_HEADER: u32 = 0x0201_4b50;

const ZIP_END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;

const ZIP_UTF8_FLAG: u16 = 1 << 11;

const ZIP_LOCAL_HEADER_LEN: u64 = 30;
const ZIP_CENTRAL_HEADER_LEN: u64 = 46;
const ZIP_EOCD_LEN: u64 = 22;
pub(super) const ZIP_V1_MAX_BYTES: u64 = u32::MAX as u64;
pub(super) const ZIP_V1_LIMIT_ERROR: &str = "ZIP V1 不支持 Zip64，请分批导出";

struct ZipCentralEntry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

#[derive(Debug, Default)]
pub(super) struct ZipStoreLayout {
    central_offset: u64,
    central_size: u64,
    entry_count: u16,
}

impl ZipStoreLayout {
    pub(super) fn total_bytes(&self) -> u64 {
        self.central_offset + self.central_size + ZIP_EOCD_LEN
    }
}

/// Validates this writer's STORE layout from UTF-8 names and exact, decoded byte lengths.
/// Callers can check sizes before buffering entries; the writer also checks the final manifest
/// and entries together before opening the destination.
pub(super) fn validate_zip_store_layout<'a>(
    entries: impl IntoIterator<Item = (&'a str, u64)>,
) -> Result<ZipStoreLayout, String> {
    let mut layout = ZipStoreLayout::default();
    for (name, size) in entries {
        let name_len = name.len() as u64;
        if name_len > u64::from(u16::MAX) {
            return Err(format!(
                "ZIP 条目路径过长：UTF-8 文件名有 {name_len} 字节，上限 {} 字节",
                u16::MAX
            ));
        }
        if size > ZIP_V1_MAX_BYTES {
            return Err(format!(
                "{ZIP_V1_LIMIT_ERROR}；单个条目 {name} 有 {size} 字节，需取消勾选该超限条目"
            ));
        }
        layout.entry_count = layout
            .entry_count
            .checked_add(1)
            .ok_or_else(|| format!("{ZIP_V1_LIMIT_ERROR}；条目数量超过 {}", u16::MAX))?;
        layout.central_offset += ZIP_LOCAL_HEADER_LEN + name_len + size;
        layout.central_size += ZIP_CENTRAL_HEADER_LEN + name_len;
        let archive_size = layout.total_bytes();
        if archive_size > ZIP_V1_MAX_BYTES {
            return Err(format!(
                "{ZIP_V1_LIMIT_ERROR}；含目录与尾记录共 {archive_size} 字节，上限 {ZIP_V1_MAX_BYTES} 字节"
            ));
        }
    }
    Ok(layout)
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

// The table-driven / SIMD CRC-32 already linked in through png and flate2; the bit-by-bit loop it
// replaces was the slowest step of exporting and importing large session archives.
fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

pub(super) fn write_zip_store(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
    let layout = validate_zip_store_layout(
        entries
            .iter()
            .map(|(name, data)| (name.as_str(), data.len() as u64)),
    )?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("创建导出目录失败 {}: {err}", parent.display()))?;
    }
    let mut file = fs::File::create(path)
        .map_err(|err| format!("创建 zip 文件失败 {}: {err}", path.display()))?;
    let mut central_entries = Vec::with_capacity(entries.len());
    let mut local_offset = 0u32;
    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        let size = data.len() as u32;
        let crc = crc32(data);
        write_u32(&mut file, ZIP_LOCAL_FILE_HEADER)?;
        write_u16(&mut file, 20)?;
        write_u16(&mut file, ZIP_UTF8_FLAG)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 33)?;
        write_u32(&mut file, crc)?;
        write_u32(&mut file, size)?;
        write_u32(&mut file, size)?;
        write_u16(&mut file, name_bytes.len() as u16)?;
        write_u16(&mut file, 0)?;
        file.write_all(name_bytes)
            .map_err(|err| format!("写入 zip 条目名失败: {err}"))?;
        file.write_all(data)
            .map_err(|err| format!("写入 zip 条目失败: {err}"))?;
        central_entries.push(ZipCentralEntry {
            name: name.clone(),
            crc,
            size,
            offset: local_offset,
        });
        local_offset += ZIP_LOCAL_HEADER_LEN as u32 + name_bytes.len() as u32 + size;
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
    write_u32(&mut file, ZIP_END_OF_CENTRAL_DIRECTORY)?;
    write_u16(&mut file, 0)?;
    write_u16(&mut file, 0)?;
    write_u16(&mut file, layout.entry_count)?;
    write_u16(&mut file, layout.entry_count)?;
    write_u32(&mut file, layout.central_size as u32)?;
    write_u32(&mut file, layout.central_offset as u32)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn crc32_matches_the_standard_check_value() {
        // CRC-32/ISO-HDLC check value, the CRC the zip format uses.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn zip_v1_checks_full_layout_at_the_byte_limit() {
        let empty = validate_zip_store_layout([]).unwrap();
        assert_eq!(empty.total_bytes(), 22);
        let manifest = ("manifest.json", 17);
        let name = "sessions/会话.jsonl";
        let headers_and_other_data = validate_zip_store_layout([manifest, (name, 0)])
            .unwrap()
            .total_bytes();
        let entry_max = u64::from(u32::MAX) - headers_and_other_data;
        let at_limit = validate_zip_store_layout([manifest, (name, entry_max)]).unwrap();
        assert_eq!(at_limit.total_bytes(), u64::from(u32::MAX));
        for size in [entry_max + 1, u64::MAX] {
            let error = validate_zip_store_layout([manifest, (name, size)]).unwrap_err();
            assert!(error.contains("请分批导出"), "{error}");
        }
    }

    #[test]
    fn zip_v1_checks_entry_count_before_writing() {
        let max_count = usize::from(u16::MAX);
        let layout =
            validate_zip_store_layout(std::iter::repeat_n(("entry", 0), max_count)).unwrap();
        assert_eq!(layout.entry_count, u16::MAX);
        let error = validate_zip_store_layout(std::iter::repeat_n(("entry", 0), max_count + 1))
            .unwrap_err();
        assert!(
            error.contains("条目数量") && error.contains("请分批导出"),
            "{error}"
        );
    }

    #[test]
    fn zip_v1_layout_matches_written_headers_and_does_not_truncate_on_invalid_input() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let target = std::env::temp_dir().join(format!("codex-switch-zip-layout-{stamp}.zip"));
        let entries = [
            ("manifest.json".to_string(), b"{}".to_vec()),
            ("sessions/历史.jsonl".to_string(), b"rollout data".to_vec()),
        ];
        let layout = validate_zip_store_layout(
            entries
                .iter()
                .map(|(name, data)| (name.as_str(), data.len() as u64)),
        )
        .unwrap();
        write_zip_store(&target, &entries).unwrap();
        let original = fs::read(&target).unwrap();
        assert_eq!(original.len() as u64, layout.total_bytes());
        let eocd = original.len() - 22;
        assert_eq!(
            read_u32_at(&original, eocd).unwrap(),
            ZIP_END_OF_CENTRAL_DIRECTORY
        );
        assert_eq!(
            read_u32_at(&original, eocd + 16).unwrap() as u64,
            layout.central_offset
        );
        assert_eq!(
            read_u32_at(&original, eocd + 12).unwrap() as u64,
            layout.central_size
        );
        assert_eq!(
            read_u16_at(&original, eocd + 10).unwrap(),
            layout.entry_count
        );
        let second = layout.central_offset as usize + 46 + entries[0].0.len();
        assert_eq!(
            read_u16_at(&original, second + 28).unwrap() as usize,
            entries[1].0.len()
        );
        assert_eq!(
            read_u32_at(&original, second + 42).unwrap() as usize,
            30 + entries[0].0.len() + entries[0].1.len()
        );

        let long_name = "字".repeat(usize::from(u16::MAX) / 3 + 1);
        let invalid = [entries[0].clone(), (long_name, vec![])];
        let error = write_zip_store(&target, &invalid).unwrap_err();
        assert!(error.contains("UTF-8 文件名"), "{error}");
        assert_eq!(fs::read(&target).unwrap(), original);
        fs::remove_file(target).unwrap();
    }
}
