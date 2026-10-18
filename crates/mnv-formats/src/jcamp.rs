//! JCAMP-DX 读取（IUPAC 公开标准，重点支持 `X++(Y..Y)`）。
//!
//! 支持：
//! * `##XYDATA= (X++(Y..Y))`，含 `DIF`（差分编码）与 `DUP`（重复编码）
//! * `##XYDATA= (XY..XY)` 成对数据
//! * `##XFACTOR` / `##YFACTOR` 缩放
//!
//! 明确不支持：`SQZ`（有损压缩）。遇到时返回 `Unsupported` 并说明原因。

use mnv_core::{
    AcquisitionMeta, Axis, MnvError, Nucleus, Result, SourceInfo, Spectrum1D, Vendor,
};

use crate::{Loaded, LoadedData};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Num(f64),
    Dif,
    Dup,
}

fn tokenize(line: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    for raw in line.split([' ', ',', '\t', ';']) {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        if t.eq_ignore_ascii_case("DIF") {
            out.push(Tok::Dif);
            continue;
        }
        if t.eq_ignore_ascii_case("DUP") {
            out.push(Tok::Dup);
            continue;
        }
        if let Ok(v) = t.parse::<f64>() {
            out.push(Tok::Num(v));
        }
        // 其他无法识别的 token（如注释）直接忽略
    }
    out
}

pub fn parse(text: &str, path: Option<&str>) -> Result<Loaded> {
    let mut labels: Vec<(String, String)> = Vec::new();
    let mut data_lines: Vec<&str> = Vec::new();
    let mut in_data = false;
    let mut mode = String::new();

    for raw in text.lines() {
        let line = raw.trim_end();
        if let Some(rest) = line.strip_prefix("##") {
            if let Some((k, v)) = rest.split_once('=') {
                let key = k.trim().to_ascii_uppercase();
                let val = v.trim().to_string();
                if key.starts_with("XYDATA") || key.starts_with("DATA TABLE") || key == "DATATABLE"
                {
                    in_data = true;
                    mode = val.clone();
                    continue;
                }
                if key == "END" {
                    in_data = false;
                    continue;
                }
                if in_data {
                    continue;
                }
                labels.push((key, val));
            }
            continue;
        }
        if in_data && !line.trim().is_empty() {
            data_lines.push(line);
        }
    }

    let get = |key: &str| -> Option<&str> {
        labels
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };

    let num = |key: &str| -> Option<f64> {
        get(key).and_then(|v| {
            v.split_whitespace()
                .next()
                .and_then(|t| t.replace(',', "").parse::<f64>().ok())
        })
    };

    if mode.to_ascii_uppercase().contains("SQZ") {
        return Err(MnvError::unsupported(
            "JCAMP 文件使用 SQZ 有损压缩，本版本未实现；请用原厂软件另存为未压缩 JCAMP-DX",
        ));
    }
    if data_lines.is_empty() {
        return Err(MnvError::incomplete("JCAMP 文件中未找到数据段（##XYDATA）"));
    }

    let xfactor = num("XFACTOR").unwrap_or(1.0);
    let yfactor = num("YFACTOR").unwrap_or(1.0);
    let first_x = num("FIRSTX");
    let last_x = num("LASTX");
    let npoints = num("NPOINTS").map(|v| v.max(0.0) as usize);

    let pairs_mode = mode.to_ascii_uppercase().contains("XY..XY");
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();

    if pairs_mode {
        for line in &data_lines {
            let toks = tokenize(line);
            let mut i = 0;
            while i + 1 < toks.len() {
                if let (Tok::Num(x), Tok::Num(y)) = (toks[i], toks[i + 1]) {
                    xs.push(x * xfactor);
                    ys.push(y * yfactor);
                }
                i += 2;
            }
        }
    } else {
        let mut prev = 0.0f64;
        let mut dif = false;
        let total = data_lines.len();
        for (li, line) in data_lines.iter().enumerate() {
            let toks = tokenize(line);
            if toks.is_empty() {
                continue;
            }
            // X++(Y..Y)：每行第一个数字是该行的 X，其余为 Y
            let start = if matches!(toks.first(), Some(Tok::Num(_))) { 1 } else { 0 };
            let mut vals: Vec<f64> = Vec::new();
            let mut i = start;
            while i < toks.len() {
                match toks[i] {
                    Tok::Dif => dif = true,
                    Tok::Dup => {
                        let count = match toks.get(i + 1) {
                            Some(Tok::Num(n)) => (*n).max(0.0) as usize,
                            _ => 1,
                        };
                        for _ in 0..count {
                            vals.push(prev);
                        }
                        i += 1;
                    }
                    Tok::Num(v) => {
                        if dif {
                            prev += v;
                        } else {
                            prev = v;
                        }
                        vals.push(prev);
                    }
                }
                i += 1;
            }
            // 约定：行末的 Y 是下一行的第一个 Y
            if li + 1 < total && vals.len() > 1 {
                vals.pop();
            }
            ys.extend(vals);
        }
    }

    if ys.is_empty() {
        return Err(MnvError::incomplete("JCAMP 数据段解析后为空"));
    }

    let mut notes = Vec::new();
    // 以实际数据点数为准：个别导出器写的 NPOINTS 与实际不符
    let n = ys.len();
    if let Some(declared) = npoints {
        if declared != n {
            notes.push(format!("NPOINTS 声明 {declared}，实际解析 {n} 点，以实际为准"));
        }
    }
    if n < 2 {
        return Err(MnvError::incomplete("JCAMP 数据点不足 2 个"));
    }

    // 轴：优先用 FIRSTX/LASTX + NPOINTS 线性推算
    let (start_ppm, end_ppm) = match (first_x, last_x) {
        (Some(a), Some(b)) => (a * xfactor, b * xfactor),
        _ => (n as f64 - 1.0, 0.0),
    };
    if start_ppm < end_ppm {
        ys.reverse();
        notes.push("JCAMP 横轴为递增，已翻转为 NMR 惯例（左高右低）".into());
    }
    let xunits = get("XUNITS").unwrap_or("PPM").to_ascii_uppercase();
    if xunits != "PPM" {
        notes.push(format!("横轴单位 {xunits} 非 PPM，定标需手动调整"));
    }

    let nucleus = Nucleus::from_symbol(get(".OBSERVE NUCLEUS").unwrap_or("1H"));
    let freq = num(".OBSERVE FREQUENCY").unwrap_or(0.0);
    let start = if start_ppm < end_ppm { end_ppm } else { start_ppm };
    let end = if start_ppm < end_ppm { start_ppm } else { end_ppm };
    let sw_ppm = (start - end).abs();
    let meta = AcquisitionMeta {
        nucleus,
        spectrometer_mhz: if freq > 0.0 { freq } else { 400.0 },
        spectral_width_hz: sw_ppm * if freq > 0.0 { freq } else { 400.0 },
        carrier_ppm: (start + end) / 2.0,
        scanned_points: n,
        ..Default::default()
    };
    let mut spec = Spectrum1D::new(Axis::new(start, end, n), ys.iter().map(|v| *v as f32).collect(), meta);
    spec.source = SourceInfo {
        vendor: Vendor::Jacmp,
        path: path.map(|p| p.to_string()),
        detail: get("TITLE").map(|s| s.to_string()),
    };
    let source = spec.source.clone();
    Ok(Loaded {
        data: LoadedData::Spectrum(spec),
        source,
        title: get("TITLE")
            .map(|s| s.to_string())
            .or_else(|| path.map(|p| p.to_string()))
            .unwrap_or_else(|| "JCAMP-DX".into()),
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "##TITLE= demo
##JCAMP-DX= 5.00
##DATA TYPE= NMR SPECTRUM
##XUNITS= PPM
##YUNITS= ARBITRARY UNITS
##FIRSTX= 3.0
##LASTX= 1.0
##NPOINTS= 5
##XFACTOR= 1.0
##YFACTOR= 1.0
##.OBSERVE FREQUENCY= 400.13
##.OBSERVE NUCLEUS= ^1H
##XYDATA= (X++(Y..Y))
3.0 10 20 30 40
1.5 40 50
##END=
";

    #[test]
    fn parses_xyy_block() {
        let loaded = parse(SAMPLE, None).unwrap();
        match loaded.data {
            LoadedData::Spectrum(s) => {
                assert_eq!(s.y.len(), 5);
                assert!((s.x.start_ppm - 3.0).abs() < 1e-9);
                assert!((s.x.end_ppm - 1.0).abs() < 1e-9);
                assert_eq!(s.y[0] as i32, 10);
                assert_eq!(s.y[4] as i32, 50);
            }
            _ => panic!("应为谱图"),
        }
    }

    #[test]
    fn dif_and_dup() {
        let text = "##FIRSTX= 4.0\n##LASTX= 0.0\n##NPOINTS= 6\n##XYDATA= (X++(Y..Y))\n4.0 10 DIF 1 1\n3.0 DUP 3\n##END=\n";
        let loaded = parse(text, None).unwrap();
        match loaded.data {
            LoadedData::Spectrum(s) => {
                // 行1 去掉与下行重复的末值 → 10,11；行2 差分延续 + DUP 3 次 → 12,12,12
                assert_eq!(s.y[0] as i32, 10);
                assert_eq!(s.y[1] as i32, 11);
                assert_eq!(s.y[2] as i32, 12);
                assert_eq!(s.y.len(), 5);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn sqz_is_rejected_explicitly() {
        let text = "##XYDATA= (X++(Y..Y)) SQZ\nA@ABCD\n##END=\n";
        let e = parse(text, None).unwrap_err();
        assert!(e.to_string().contains("SQZ"));
    }
}
