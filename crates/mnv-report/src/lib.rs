//! mnv-report —— 文本报告与表格导出。
//!
//! 先做一份做到位的 JACS 风格，再加 Angew/RSC 的差异点：
//! 精度位数、J 的写法、区间连接符。模板规则外置为常量，方便用户改动。

use mnv_core::{Analysis, Multiplet, ProcOp, Spectrum1D};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalStyle {
    Jacs,
    Angew,
    Rsc,
}

impl JournalStyle {
    pub fn label(&self) -> &'static str {
        match self {
            JournalStyle::Jacs => "JACS",
            JournalStyle::Angew => "Angew",
            JournalStyle::Rsc => "RSC",
        }
    }

    pub fn decimals(&self) -> usize {
        match self {
            JournalStyle::Jacs => 2,
            JournalStyle::Angew => 2,
            JournalStyle::Rsc => 2,
        }
    }

    pub fn j_decimals(&self) -> usize {
        match self {
            JournalStyle::Jacs => 1,
            JournalStyle::Angew => 1,
            JournalStyle::Rsc => 1,
        }
    }

    /// 区间连接符：JACS/Angew 用 en dash，RSC 用“–”同样处理但 J 用括号。
    pub fn range_dash(&self) -> &'static str {
        "–"
    }
}

/// 生成谱图数据行，例如：
/// `δ 4.12 (q, J = 7.1 Hz, 2H), 2.04 (s, 3H), 1.26 (t, J = 7.1 Hz, 3H).`
pub fn data_line(analysis: &Analysis, style: JournalStyle) -> String {
    let d = style.decimals();
    let jd = style.j_decimals();
    let mut items: Vec<String> = Vec::new();

    for m in &analysis.multiplets {
        let range = m.range_text(d);
        let label = m.label.symbol();
        // H 数优先用积分区间的值（化学上正确的路径：积分驱动氢数）
        let h = format_h(integral_for(analysis, m).unwrap_or(m.integral_value));
        let j = if m.label == mnv_core::MultipletLabel::S {
            None
        } else {
            m.j_text(jd)
        };
        let body = match j {
            Some(j) => format!("{label}, J = {j} Hz, {h}"),
            None => format!("{label}, {h}"),
        };
        items.push(format!("{range} ({body})"));
    }

    if items.is_empty() {
        "δ（无多重峰数据）".to_string()
    } else {
        format!("δ {}.", items.join(", "))
    }
}

/// 找出与多重峰区间重叠的积分值。
pub fn integral_for(analysis: &Analysis, m: &Multiplet) -> Option<f32> {
    analysis
        .integrals
        .iter()
        .find(|r| {
            let (hi, lo) = (r.from_ppm.max(r.to_ppm), r.from_ppm.min(r.to_ppm));
            !(lo > m.from_ppm.max(m.to_ppm) + 0.005 || hi < m.from_ppm.min(m.to_ppm) - 0.005)
        })
        .map(|r| r.value)
}

fn format_h(v: f32) -> String {
    // 水峰/内标常见 0.1H 量级，保留一位小数比强行凑整更可信
    if v < 0.95 {
        return format!("{v:.1}H");
    }
    let r = (v * 2.0).round() / 2.0;
    if (r - r.round()).abs() < 1e-6 {
        format!("{}H", r.round() as i64)
    } else {
        format!("{r:.1}H")
    }
}

/// 峰表（Markdown）。
pub fn peak_table_markdown(spec: &Spectrum1D, analysis: &Analysis) -> String {
    let mut s = String::new();
    s.push_str("| δ (ppm) | 高度 | 半高宽 (Hz) | 类型 |\n");
    s.push_str("| --- | --- | --- | --- |\n");
    for i in analysis.sorted_peak_indices() {
        let p = &analysis.peaks[i];
        s.push_str(&format!(
            "| {:.3} | {:.1} | {:.2} | {} |\n",
            p.ppm,
            p.height,
            p.width_hz,
            p.kind.label()
        ));
    }
    let _ = spec;
    s
}

/// 峰表（CSV，给 Origin / Excel）。
pub fn peak_table_csv(analysis: &Analysis) -> String {
    let mut s = String::from("ppm,height,width_hz,kind,assigned_h\n");
    for i in analysis.sorted_peak_indices() {
        let p = &analysis.peaks[i];
        s.push_str(&format!(
            "{:.4},{:.4},{:.3},{},{}\n",
            p.ppm,
            p.height,
            p.width_hz,
            p.kind.label(),
            p.assigned_h.map(|h| format!("{h}")).unwrap_or_default()
        ));
    }
    s
}

/// 处理参数表（可复现凭据）。
pub fn processing_table(history: &[ProcOp]) -> String {
    let mut s = String::new();
    for (i, op) in history.iter().enumerate() {
        s.push_str(&format!("{:>2}. {}\n", i + 1, op.describe()));
    }
    s
}

/// 完整的 SI 风格文本块：标题 + 数据行 + 参数表。
pub fn si_block(spec: &Spectrum1D, analysis: &Analysis, style: JournalStyle) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "{} NMR ({:.0} MHz, {}) {} \n",
        spec.meta.nucleus.display(),
        spec.meta.spectrometer_mhz,
        spec.meta.solvent.clone().unwrap_or_else(|| "—".into()),
        data_line(analysis, style)
    ));
    s.push_str("\n处理参数：\n");
    s.push_str(&processing_table(&spec.history));
    s
}

/// 单条多重峰的文本，H 数优先取重叠的积分值。
pub fn multiplet_line(analysis: &Analysis, m: &Multiplet) -> String {
    let h = integral_for(analysis, m).unwrap_or(m.integral_value);
    let body = match m.j_text(1) {
        Some(j) => format!("{}, J = {} Hz, {}", m.label.symbol(), j, format_h(h)),
        None => format!("{}, {}", m.label.symbol(), format_h(h)),
    };
    format!("{} ({})", m.range_text(2), body)
}

/// 单条多重峰的文本（右键“复制此峰”）。
pub fn multiplet_text(m: &Multiplet) -> String {
    let body = match m.j_text(1) {
        Some(j) => format!("{}, J = {} Hz, {}", m.label.symbol(), j, format_h(m.integral_value)),
        None => format!("{}, {}", m.label.symbol(), format_h(m.integral_value)),
    };
    format!("{} ({})", m.range_text(2), body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnv_core::{Multiplet, MultipletLabel};

    fn fake_analysis() -> Analysis {
        Analysis {
            multiplets: vec![
                Multiplet {
                    from_ppm: 4.14,
                    to_ppm: 4.10,
                    peaks: vec![],
                    j_values_hz: vec![7.1, 7.1, 7.1],
                    integral: 1.0,
                    integral_value: 2.0,
                    label: MultipletLabel::Q,
                    assignment: None,
                    fit_residual_hz: 0.05,
                },
                Multiplet {
                    from_ppm: 2.04,
                    to_ppm: 2.04,
                    peaks: vec![],
                    j_values_hz: vec![],
                    integral: 1.0,
                    integral_value: 3.0,
                    label: MultipletLabel::S,
                    assignment: None,
                    fit_residual_hz: 0.0,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn jacs_line_matches_convention() {
        let text = data_line(&fake_analysis(), JournalStyle::Jacs);
        assert_eq!(text, "δ 4.14–4.10 (q, J = 7.1 Hz, 2H), 2.04 (s, 3H).");
    }

    #[test]
    fn csv_has_header() {
        let csv = peak_table_csv(&fake_analysis());
        assert!(csv.starts_with("ppm,height,width_hz,kind,assigned_h"));
    }
}
