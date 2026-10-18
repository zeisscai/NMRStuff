//! Bruker TopSpin 数据读取（公开目录结构与参数文件格式）。
//!
//! 目录约定（事实公开，nmrglue / TopSpin 手册 / CEMHTI 文档均有描述）：
//! ```text
//! <expno>/acqus            采集参数（纯文本，##KEY= value）
//! <expno>/fid              时域数据（int32 交错复数）
//! <expno>/pdata/1/procs    处理参数
//! <expno>/pdata/1/1r       处理后的实部谱（int32）
//! ```
//!
//! 字段含义：
//! * `SW_h` 谱宽（Hz）、`SFO1` 观测频率（MHz）、`O1` 载波偏移（Hz，相对 SFO1）
//! * `BYTORDA` 原始数据字节序（0 = little endian）
//! * `GRPDLY` 数字滤波群延迟（采样点）
//! * `NUC1` / `SOLVENT` / `PULPROG` 核种 / 溶剂 / 脉冲程序

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use mnv_core::{
    AcquisitionMeta, Axis, Fid, MnvError, Nucleus, Result, SourceInfo, Spectrum1D, Vendor,
};
use num_complex::Complex32;

use crate::{Loaded, LoadedData};

#[derive(Clone, Debug, Default)]
pub struct BrukerParams {
    map: HashMap<String, String>,
}

impl BrukerParams {
    pub fn parse(text: &str) -> Self {
        let mut map: HashMap<String, String> = HashMap::new();
        let mut current: Option<String> = None;
        let mut buffer = String::new();

        for raw in text.lines() {
            let line = raw.trim_end_matches(['\r', '\n']);
            if let Some(rest) = line.strip_prefix("##") {
                // 形如 `##$TD= 32768` 或 `##.SOLVENT= <CDCl3>` 或 `##TITLE=`
                let (key_raw, value) = match rest.split_once('=') {
                    Some((k, v)) => (k, v.trim()),
                    None => continue,
                };
                let key = key_raw
                    .trim_start_matches('$')
                    .trim_start_matches('.')
                    .trim()
                    .to_string();
                if value.starts_with('<') && !value.ends_with('>') {
                    // 数组型参数可能跨行
                    current = Some(key);
                    buffer = format!("{value} ");
                } else {
                    map.insert(key, value.to_string());
                }
                continue;
            }
            if let Some(key) = current.clone() {
                buffer.push_str(line);
                buffer.push(' ');
                if buffer.contains('>') {
                    map.insert(key, buffer.trim().to_string());
                    current = None;
                    buffer.clear();
                }
            }
        }
        Self { map }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str())
    }

    pub fn text(&self, key: &str) -> Option<String> {
        self.get(key).map(|v| {
            v.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .trim()
                .to_string()
        })
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        self.text(key).and_then(|v| {
            v.split_whitespace()
                .next()
                .and_then(|t| t.parse::<f64>().ok())
        })
    }

    pub fn usize(&self, key: &str) -> Option<usize> {
        self.f64(key).map(|v| v as usize)
    }
}

/// 在给定路径及其上级目录中寻找 `acqus`。
pub fn find_acqus(start: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let base = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    candidates.push(base.join("acqus"));
    candidates.push(base.join("../acqus"));
    candidates.push(base.join("pdata/1/../../acqus"));
    candidates.push(base.join("../../acqus"));
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| normalize(&p))
}

fn normalize(p: &Path) -> PathBuf {
    // 不用 fs::canonicalize：避免 Windows UNC 前缀污染显示路径
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 读取一个 Bruker 数据集目录（或其中任意文件）。
pub fn read(path: &Path) -> Result<Loaded> {
    let acqus_path = find_acqus(path).ok_or_else(|| {
        MnvError::incomplete(format!(
            "在 {} 及其上级目录中未找到 acqus 参数文件，无法确定采集参数",
            path.display()
        ))
    })?;
    let dir = acqus_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let text = fs::read_to_string(&acqus_path)
        .map_err(|e| MnvError::parse(format!("读取 acqus 失败：{e}")))?;
    let params = BrukerParams::parse(&text);

    let sfo = params
        .f64("SFO1")
        .or_else(|| params.f64("BF1"))
        .ok_or_else(|| MnvError::incomplete("acqus 缺少 SFO1/BF1，无法确定观测频率"))?;
    let sw = params
        .f64("SW_h")
        .or_else(|| params.f64("SW").map(|v| v * sfo))
        .ok_or_else(|| MnvError::incomplete("acqus 缺少 SW_h，无法确定谱宽"))?;
    let o1 = params.f64("O1").unwrap_or(0.0);
    let nucleus = Nucleus::from_symbol(&params.text("NUC1").unwrap_or_else(|| "1H".into()));

    let mut notes = Vec::new();
    let fid_path = dir.join("fid");
    let title = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Bruker".into());

    let source = SourceInfo {
        vendor: Vendor::Bruker,
        path: Some(dir.to_string_lossy().to_string()),
        detail: params.text("PULPROG"),
    };

    if fid_path.is_file() {
        let bytes = fs::read(&fid_path)?;
        let big_endian = params.usize("BYTORDA").unwrap_or(0) == 1;
        let ints = bytes_to_i32(&bytes, big_endian);
        if ints.len() < 4 {
            return Err(MnvError::incomplete(format!(
                "fid 数据过短（{} 字节）",
                bytes.len()
            )));
        }
        let n = ints.len() / 2;
        let data: Vec<Complex32> = (0..n)
            .map(|i| Complex32::new(ints[2 * i] as f32, ints[2 * i + 1] as f32))
            .collect();

        let meta = AcquisitionMeta {
            nucleus,
            spectrometer_mhz: sfo,
            spectral_width_hz: sw,
            carrier_ppm: if sfo.abs() > 1e-12 { o1 / sfo } else { 0.0 },
            solvent: params.text("SOLVENT"),
            pulse_program: params.text("PULPROG"),
            scanned_points: n,
            group_delay: params.f64("GRPDLY"),
            temperature_k: params.f64("TE"),
            operator: params.text("OPERATOR"),
            comment: params
                .text("TITLE")
                .filter(|s| !s.trim().is_empty())
                .or_else(|| Some(title.clone())),
        };

        let mut fid = Fid::new(data, meta);
        fid.source = source.clone();
        return Ok(Loaded {
            data: LoadedData::Fid(fid),
            source,
            title,
            notes,
        });
    }

    // 回退：读取已处理的 1r 谱
    if let Some(spec) = read_processed_1r(&dir, &params, &nucleus, sfo, &mut notes)? {
        return Ok(Loaded {
            data: LoadedData::Spectrum(spec),
            source,
            title,
            notes,
        });
    }

    Err(MnvError::incomplete(format!(
        "{} 中既没有 fid，也没有 pdata/1/1r",
        dir.display()
    )))
}

fn bytes_to_i32(bytes: &[u8], big_endian: bool) -> Vec<i32> {
    let n = bytes.len() / 4;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = &bytes[i * 4..i * 4 + 4];
        let v = if big_endian {
            i32::from_be_bytes([b[0], b[1], b[2], b[3]])
        } else {
            i32::from_le_bytes([b[0], b[1], b[2], b[3]])
        };
        out.push(v);
    }
    out
}

fn read_processed_1r(
    dir: &Path,
    acqus: &BrukerParams,
    nucleus: &Nucleus,
    sfo_acqus: f64,
    notes: &mut Vec<String>,
) -> Result<Option<Spectrum1D>> {
    let pdata = dir.join("pdata/1");
    let one_r = pdata.join("1r");
    if !one_r.is_file() {
        return Ok(None);
    }
    let procs = pdata
        .join("procs")
        .is_file()
        .then(|| fs::read_to_string(pdata.join("procs")).ok())
        .flatten()
        .map(|t| BrukerParams::parse(&t))
        .unwrap_or_default();

    let sf = procs.f64("SF").unwrap_or(sfo_acqus);
    let sw_p = procs.f64("SW_p").unwrap_or(10.0);
    let offset = procs.f64("OFFSET").unwrap_or(0.0);
    let bytordp = procs.usize("BYTORDP").unwrap_or(0) == 1;
    let nc = procs.text("NC_proc").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);

    let bytes = fs::read(&one_r)?;
    let ints = bytes_to_i32(&bytes, bytordp);
    let scale = 2f32.powi(nc);
    let y: Vec<f32> = ints.iter().map(|v| *v as f32 * scale).collect();
    if y.is_empty() {
        return Ok(None);
    }
    notes.push("未找到 fid，改用 pdata/1/1r（已处理谱，处理链为空）".into());
    let axis = Axis::new(offset, offset - sw_p, y.len());
    let meta = AcquisitionMeta {
        nucleus: nucleus.clone(),
        spectrometer_mhz: sf,
        spectral_width_hz: sw_p * sf,
        carrier_ppm: offset - sw_p / 2.0,
        solvent: acqus.text("SOLVENT"),
        pulse_program: acqus.text("PULPROG"),
        scanned_points: y.len(),
        ..Default::default()
    };
    let mut spec = Spectrum1D::new(axis, y, meta);
    spec.source = SourceInfo {
        vendor: Vendor::Bruker,
        path: Some(one_r.to_string_lossy().to_string()),
        detail: Some("pdata/1/1r".into()),
    };
    Ok(Some(spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_acqus_scalars() {
        let text = "##TITLE= test\n##$TD= 32768\n##$SW_h= 5200.0\n##$NUC1= <1H>\n##$SOLVENT= <CDCl3>\n##$GRPDLY= 0\n##END=\n";
        let p = BrukerParams::parse(text);
        assert_eq!(p.usize("TD"), Some(32768));
        assert_eq!(p.f64("SW_h"), Some(5200.0));
        assert_eq!(p.text("NUC1").as_deref(), Some("1H"));
        assert_eq!(p.text("SOLVENT").as_deref(), Some("CDCl3"));
    }

    #[test]
    fn parse_multiline_array() {
        let text = "##$D= (\n1 2 3\n)\n##$SFO1= 400.13\n";
        let p = BrukerParams::parse(text);
        assert_eq!(p.f64("SFO1"), Some(400.13));
        assert!(p.get("D").is_some());
    }

    #[test]
    fn big_endian_bytes() {
        let b = [0x00, 0x00, 0x01, 0x00];
        assert_eq!(bytes_to_i32(&b, true)[0], 256);
        assert_eq!(bytes_to_i32(&b, false)[0], 65536);
    }
}
