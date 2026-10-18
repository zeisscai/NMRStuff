//! 格式自动识别与分派。
//!
//! 识别优先级：目录结构（Bruker）> 扩展名 > 内容嗅探。
//! 无法识别时返回带“下一步怎么做”的错误信息。

use std::fs;
use std::path::Path;

use mnv_core::{
    AcquisitionMeta, Axis, MnvError, Nucleus, Result, SourceInfo, Spectrum1D, Vendor,
};

use crate::{bruker, jcamp, Loaded, LoadedData};

pub fn load_path(path: &str) -> Result<Loaded> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(MnvError::incomplete(format!("路径不存在：{path}")));
    }
    if p.is_dir() {
        return load_dir(p);
    }
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "jdx" | "dx" | "jcamp" => {
            let text = fs::read_to_string(p)
                .map_err(|e| MnvError::parse(format!("读取文本失败：{e}")))?;
            jcamp::parse(&text, Some(path))
        }
        "csv" | "tsv" | "txt" | "dat" => {
            let text = fs::read_to_string(p)
                .map_err(|e| MnvError::parse(format!("读取文本失败：{e}")))?;
            read_columns(&text, Some(path))
        }
        "fid" | "1r" | "ser" | "acqus" | "procs" => bruker::read(p),
        _ => {
            // 内容嗅探：JCAMP 以 `##TITLE=` 开头
            if let Ok(text) = fs::read_to_string(p) {
                if text.trim_start().starts_with("##") {
                    return jcamp::parse(&text, Some(path));
                }
                if looks_numeric(&text) {
                    return read_columns(&text, Some(path));
                }
            }
            Err(MnvError::unsupported(format!(
                "无法识别扩展名 .{ext}。支持的格式：Bruker 数据集目录、JCAMP-DX(.jdx/.dx)、CSV/文本两列"
            )))
        }
    }
}

fn load_dir(dir: &Path) -> Result<Loaded> {
    if bruker::find_acqus(dir).is_some() {
        return bruker::read(dir);
    }
    if dir.join("procpar").is_file() {
        return Err(MnvError::unsupported(
            "检测到 Varian/Agilent 数据集（procpar）。该格式尚未支持；\
             临时方案：在 VNMRJ 中导出为 JCAMP-DX 后再导入",
        ));
    }
    // 目录中若只有一个 JCAMP/CSV，顺手读入
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if let Some(ext) = p.extension().map(|s| s.to_string_lossy().to_ascii_lowercase()) {
                if matches!(ext.as_str(), "jdx" | "dx" | "csv" | "txt" | "dat") {
                    candidates.push(p);
                }
            }
        }
    }
    candidates.sort();
    if candidates.len() == 1 {
        return load_path(&candidates[0].to_string_lossy());
    }
    Err(MnvError::unsupported(format!(
        "目录 {} 中未找到 acqus / fid，也没有唯一的 JCAMP/CSV 文件",
        dir.display()
    )))
}

fn looks_numeric(text: &str) -> bool {
    text.lines()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| {
            let cols: Vec<&str> = l.split([',', ';', '\t', ' ']).filter(|s| !s.is_empty()).collect();
            cols.len() >= 2 && cols[0].parse::<f64>().is_ok() && cols[1].parse::<f64>().is_ok()
        })
        .unwrap_or(false)
}

/// 两列文本（CSV/TSV/空白分隔）。第一列为 ppm，第二列为强度。
pub fn read_columns(text: &str, path: Option<&str>) -> Result<Loaded> {
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with("//") {
            continue;
        }
        let cols: Vec<&str> = l
            .split([',', ';', '\t', ' '])
            .filter(|s| !s.is_empty())
            .collect();
        if cols.len() < 2 {
            continue;
        }
        let (Ok(x), Ok(y)) = (cols[0].parse::<f64>(), cols[1].parse::<f64>()) else {
            continue;
        };
        xs.push(x);
        ys.push(y);
    }
    if ys.len() < 4 {
        return Err(MnvError::incomplete(
            "两列文本中有效数据少于 4 行（需要 ppm 与强度两列）",
        ));
    }

    let mut notes = Vec::new();
    let ascending = xs.last().copied().unwrap_or(0.0) > xs[0];
    if ascending {
        xs.reverse();
        ys.reverse();
        notes.push("输入为 ppm 递增，已翻转为左高右低".into());
    }
    let n = ys.len();
    let span = (xs[0] - xs[n - 1]).abs();
    let meta = AcquisitionMeta {
        nucleus: Nucleus::H1,
        spectrometer_mhz: 400.0,
        spectral_width_hz: span * 400.0,
        carrier_ppm: (xs[0] + xs[n - 1]) / 2.0,
        scanned_points: n,
        ..Default::default()
    };
    notes.push("两列文本不含采集参数：观测频率按 400 MHz 假定，J 值需据此修正".into());

    let mut spec = Spectrum1D::new(
        Axis::new(xs[0], xs[n - 1], n),
        ys.iter().map(|v| *v as f32).collect(),
        meta,
    );
    spec.source = SourceInfo {
        vendor: Vendor::Csv,
        path: path.map(|p| p.to_string()),
        detail: None,
    };
    let source = spec.source.clone();
    Ok(Loaded {
        data: LoadedData::Spectrum(spec),
        source,
        title: path.unwrap_or("两列文本").to_string(),
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_two_columns() {
        let text = "ppm,intensity\n3.0,10\n2.0,20\n1.0,30\n0.0,40\n";
        let l = read_columns(text, None).unwrap();
        match l.data {
            LoadedData::Spectrum(s) => {
                assert_eq!(s.y.len(), 4);
                assert!((s.x.start_ppm - 3.0).abs() < 1e-9);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn ascending_csv_is_flipped() {
        let text = "0.0,40\n1.0,30\n2.0,20\n3.0,10\n";
        let l = read_columns(text, None).unwrap();
        match l.data {
            LoadedData::Spectrum(s) => {
                assert!(s.x.start_ppm > s.x.end_ppm);
                assert!((s.y[0] - 10.0).abs() < 1e-3);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn nonexistent_path_message() {
        let e = load_path("Z:/nope/none").unwrap_err();
        assert!(e.to_string().contains("不存在"));
    }
}
