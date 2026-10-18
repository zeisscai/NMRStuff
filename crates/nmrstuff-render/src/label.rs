//! 峰标注的简易避让布局。
//!
//! 目标不是“最优排版”，而是**不重叠、可预测**：按 x 排序后贪心放置，
//! 若与已放置标签水平重叠就上移一格。

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelBox {
    /// 文本左下角
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// 引线起点（峰顶）
    pub anchor_x: f32,
    pub anchor_y: f32,
}

/// 输入 `(峰 x, 峰顶 y, 文本宽, 文本高)`，返回放置好的标签框。
pub fn layout_peak_labels(
    candidates: &[(f32, f32, f32, f32)],
    top_limit: f32,
    right_limit: f32,
    gap: f32,
) -> Vec<LabelBox> {
    let mut ordered: Vec<usize> = (0..candidates.len()).collect();
    ordered.sort_by(|a, b| {
        candidates[*a]
            .0
            .partial_cmp(&candidates[*b].0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut placed: Vec<LabelBox> = Vec::with_capacity(candidates.len());
    for idx in ordered {
        let (ax, ay, w, h) = candidates[idx];
        let mut x = (ax - w * 0.5).clamp(0.0, (right_limit - w).max(0.0));
        let mut y = ay - h - 4.0;

        // 与已放置标签冲突则上移
        let mut guard = 0;
        while placed.iter().any(|p| overlaps(p, x, y, w, h)) && guard < 64 {
            y -= h + gap;
            guard += 1;
        }
        if y < top_limit {
            // 放不下就退回到峰顶下方，保持可见
            y = (ay + 4.0).min(top_limit + 400.0);
        }
        // 修正 x 越界
        if x + w > right_limit {
            x = (right_limit - w).max(0.0);
        }
        placed.push(LabelBox {
            x,
            y,
            w,
            h,
            anchor_x: ax,
            anchor_y: ay,
        });
    }
    placed
}

fn overlaps(p: &LabelBox, x: f32, y: f32, w: f32, h: f32) -> bool {
    !(x > p.x + p.w || x + w < p.x || y > p.y + p.h || y + h < p.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_labels_overlap() {
        let cands: Vec<(f32, f32, f32, f32)> =
            (0..8).map(|i| (40.0 + i as f32 * 3.0, 100.0, 30.0, 10.0)).collect();
        let boxes = layout_peak_labels(&cands, 0.0, 400.0, 2.0);
        assert_eq!(boxes.len(), 8);
        for i in 0..boxes.len() {
            for j in i + 1..boxes.len() {
                let a = boxes[i];
                let b = boxes[j];
                let over = !(a.x > b.x + b.w || a.x + a.w < b.x || a.y > b.y + b.h || a.y + a.h < b.y);
                assert!(!over, "box {i} 与 {j} 重叠");
            }
        }
    }

    #[test]
    fn stays_within_right_limit() {
        let cands = vec![(395.0, 100.0, 60.0, 10.0)];
        let boxes = layout_peak_labels(&cands, 0.0, 400.0, 2.0);
        assert!(boxes[0].x + boxes[0].w <= 400.0 + 1e-3);
    }
}
