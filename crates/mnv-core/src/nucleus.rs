use serde::{Deserialize, Serialize};

/// 观测核种。
///
/// 只保留常用核；未知核走 `Other` 分支，保证新格式不会导致解析失败。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Nucleus {
    H1,
    H2,
    C13,
    N15,
    F19,
    P31,
    Si29,
    B11,
    O17,
    Other(String),
}

impl Default for Nucleus {
    fn default() -> Self {
        Nucleus::H1
    }
}

impl Nucleus {
    pub fn symbol(&self) -> String {
        match self {
            Nucleus::H1 => "1H".into(),
            Nucleus::H2 => "2H".into(),
            Nucleus::C13 => "13C".into(),
            Nucleus::N15 => "15N".into(),
            Nucleus::F19 => "19F".into(),
            Nucleus::P31 => "31P".into(),
            Nucleus::Si29 => "29Si".into(),
            Nucleus::B11 => "11B".into(),
            Nucleus::O17 => "17O".into(),
            Nucleus::Other(s) => s.clone(),
        }
    }

    /// 显示用符号，带上标（PDF/报告中通常需要斜体核种，此处仅作文本标注）。
    pub fn display(&self) -> String {
        match self {
            Nucleus::H1 => "¹H".into(),
            Nucleus::H2 => "²H".into(),
            Nucleus::C13 => "¹³C".into(),
            Nucleus::N15 => "¹⁵N".into(),
            Nucleus::F19 => "¹⁹F".into(),
            Nucleus::P31 => "³¹P".into(),
            Nucleus::Si29 => "²⁹Si".into(),
            Nucleus::B11 => "¹¹B".into(),
            Nucleus::O17 => "¹⁷O".into(),
            Nucleus::Other(s) => s.clone(),
        }
    }

    /// 解析 `1H` / `13C` / `^1H` / `H1` 等写法。
    pub fn from_symbol(raw: &str) -> Self {
        let s: String = raw
            .trim()
            .trim_matches(|c| c == '<' || c == '>' || c == '"')
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '^')
            .collect();
        let upper = s.to_ascii_uppercase();
        match upper.as_str() {
            "1H" | "H1" | "H" | "PROTON" => Nucleus::H1,
            "2H" | "H2" | "D" => Nucleus::H2,
            "13C" | "C13" | "C" | "CARBON" => Nucleus::C13,
            "15N" | "N15" | "N" => Nucleus::N15,
            "19F" | "F19" | "F" => Nucleus::F19,
            "31P" | "P31" | "P" => Nucleus::P31,
            "29SI" | "SI29" => Nucleus::Si29,
            "11B" | "B11" => Nucleus::B11,
            "17O" | "O17" => Nucleus::O17,
            _ => Nucleus::Other(s),
        }
    }

    /// 旋磁比（MHz/T，IUPAC 公开数据）。用于从其他核的频率推算观测频率。
    pub fn gamma_mhz_per_t(&self) -> Option<f64> {
        Some(match self {
            Nucleus::H1 => 42.577_478_92,
            Nucleus::H2 => 6.535_902,
            Nucleus::C13 => 10.708_393_6,
            Nucleus::N15 => -4.316_996,
            Nucleus::F19 => 40.077_571,
            Nucleus::P31 => 17.235_177,
            Nucleus::Si29 => -8.465_412,
            Nucleus::B11 => 13.662_850,
            Nucleus::O17 => -5.771_800,
            Nucleus::Other(_) => return None,
        })
    }

    /// 常规一维谱的默认显示窗口（ppm），仅作为新建视图的初值。
    pub fn default_range(&self) -> (f64, f64) {
        match self {
            Nucleus::H1 => (12.0, -1.0),
            Nucleus::C13 => (220.0, -20.0),
            Nucleus::F19 => (-40.0, -160.0),
            Nucleus::P31 => (60.0, -40.0),
            Nucleus::N15 => (300.0, 0.0),
            Nucleus::Si29 => (80.0, -80.0),
            Nucleus::B11 => (100.0, -100.0),
            Nucleus::O17 => (900.0, -100.0),
            Nucleus::H2 => (12.0, -1.0),
            Nucleus::Other(_) => (10.0, 0.0),
        }
    }

    /// 13C 类实验一般不积分（NOE 导致面积不成比例），用于 UI 默认行为。
    pub fn is_quantitative_by_area(&self) -> bool {
        matches!(self, Nucleus::H1 | Nucleus::F19)
    }
}

impl std::fmt::Display for Nucleus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.symbol())
    }
}
