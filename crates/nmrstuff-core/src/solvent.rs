//! 氘代溶剂残余峰参考表。
//!
//! 数据来源：Gottlieb, H. E.; Kotlyar, V.; Nudelman, A. *J. Org. Chem.* **1997**, 62,
//! 7512–7515（公开文献的化学位移事实）。本表为按该文献常见条目逐项录入的自建表，
//! 未复制任何商业软件的数据文件。

use crate::nucleus::Nucleus;

#[derive(Clone, Debug)]
pub struct Solvent {
    /// 用于参数文件的短名，如 `CDCl3`
    pub name: &'static str,
    /// 界面显示名
    pub label: &'static str,
    /// 残余 ¹H 峰（ppm，可能多个）
    pub h1: &'static [f64],
    /// 残余 ¹³C 峰（ppm，可能多个）
    pub c13: &'static [f64],
}

pub static SOLVENTS: &[Solvent] = &[
    Solvent { name: "CDCl3", label: "氯仿-d (CDCl₃)", h1: &[7.26], c13: &[77.16] },
    Solvent { name: "DMSO", label: "二甲亚砜-d6 (DMSO-d₆)", h1: &[2.50], c13: &[39.52] },
    Solvent { name: "D2O", label: "重水 (D₂O)", h1: &[4.79], c13: &[] },
    Solvent { name: "MeOD", label: "甲醇-d4 (CD₃OD)", h1: &[3.31], c13: &[49.00] },
    Solvent { name: "CD3CN", label: "乙腈-d3 (CD₃CN)", h1: &[1.94], c13: &[1.32, 118.26] },
    Solvent { name: "C6D6", label: "苯-d6 (C₆D₆)", h1: &[7.16], c13: &[128.06] },
    Solvent { name: "Acetone", label: "丙酮-d6 (acetone-d₆)", h1: &[2.05], c13: &[29.84, 206.26] },
    Solvent { name: "THF", label: "四氢呋喃-d8 (THF-d₈)", h1: &[1.73, 3.58], c13: &[25.37, 67.57] },
    Solvent { name: "Toluene", label: "甲苯-d8 (toluene-d₈)", h1: &[2.09, 6.98, 7.09], c13: &[20.43, 125.49, 128.33, 129.24, 137.86] },
    Solvent { name: "Pyridine", label: "吡啶-d5 (pyridine-d₅)", h1: &[7.22, 7.58, 8.74], c13: &[123.87, 135.91, 149.90] },
    Solvent { name: "DCM", label: "二氯甲烷-d2 (CD₂Cl₂)", h1: &[5.32], c13: &[53.84] },
    Solvent { name: "DMF", label: "N,N-二甲基甲酰胺-d7 (DMF-d₇)", h1: &[2.75, 2.92, 8.03], c13: &[30.10, 35.20, 162.70] },
    Solvent { name: "AcOH", label: "乙酸-d4 (CD₃COOD)", h1: &[2.04], c13: &[20.00, 178.00] },
    Solvent { name: "Methanol", label: "甲醇-d4（残余 OH）", h1: &[3.31], c13: &[49.00] },
    Solvent { name: "Nitromethane", label: "硝基甲烷-d3 (CD₃NO₂)", h1: &[4.33], c13: &[62.50] },
    Solvent { name: "DCE", label: "1,2-二氯乙烷-d4 (C₂D₄Cl₂)", h1: &[3.72], c13: &[43.60] },
];

/// 常见的非溶剂参考物（内标/外标）。
pub static REFERENCES: &[(&str, Nucleus, f64)] = &[
    ("TMS", Nucleus::H1, 0.0),
    ("TMS", Nucleus::C13, 0.0),
    ("DSS", Nucleus::H1, 0.0),
    ("TSP", Nucleus::H1, 0.0),
];

pub fn by_name(name: &str) -> Option<&'static Solvent> {
    let needle = name.trim();
    SOLVENTS
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(needle))
}

/// 按前几位字符模糊匹配（Bruker 参数里可能写 `CDCl3` / `DMSO`）。
pub fn guess(name: &str) -> Option<&'static Solvent> {
    let n = name.trim().to_ascii_uppercase();
    if n.is_empty() {
        return None;
    }
    SOLVENTS.iter().find(|s| {
        let sn = s.name.to_ascii_uppercase();
        n == sn || n.starts_with(&sn) || sn.starts_with(&n)
    })
}

/// 给出某核种的参考位移列表（ppm, 溶剂名）。
pub fn reference_shifts(nucleus: &Nucleus) -> Vec<(f64, &'static str)> {
    let mut out = Vec::new();
    for s in SOLVENTS {
        let list = match nucleus {
            Nucleus::H1 | Nucleus::H2 => s.h1,
            Nucleus::C13 => s.c13,
            _ => &[],
        };
        for v in list {
            out.push((*v, s.name));
        }
    }
    out
}

/// 判断给定化学位移是否落在某个溶剂残余峰附近，返回溶剂名。
pub fn match_residual(nucleus: &Nucleus, ppm: f64, tolerance_ppm: f64) -> Option<&'static str> {
    reference_shifts(nucleus)
        .into_iter()
        .filter(|(shift, _)| (shift - ppm).abs() <= tolerance_ppm)
        .min_by(|a, b| {
            (a.0 - ppm)
                .abs()
                .partial_cmp(&(b.0 - ppm).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chloroform_lookup() {
        let s = by_name("cdcl3").expect("CDCl3 应存在");
        assert!((s.h1[0] - 7.26).abs() < 1e-9);
    }

    #[test]
    fn residual_match() {
        assert_eq!(match_residual(&Nucleus::H1, 7.259, 0.05), Some("CDCl3"));
        assert_eq!(match_residual(&Nucleus::H1, 5.00, 0.05), None);
    }
}
