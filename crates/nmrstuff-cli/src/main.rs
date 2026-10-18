//! nmrstuff-cli —— 无 GUI 的批处理入口。
//!
//! 与 GUI 共享 `core/formats/proc/analysis/report`，用于验证“同一套代码两种前端”
//! 这一架构目标。

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use nmrstuff_analysis::{multiplets_from_auto, PickParams};
use nmrstuff_core::{Analysis, ProcRecipe};
use nmrstuff_formats::LoadedData;

#[derive(Parser)]
#[command(name = "nmrstuff-cli", version, about = "轻量 NMR 处理/分析命令行")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 显示数据集的元信息
    Info {
        /// Bruker 目录、JCAMP 文件或两列 CSV
        path: String,
    },
    /// 处理并输出峰表
    Proc {
        path: String,
        /// 输出 CSV 路径（缺省打印到标准输出）
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// 使用 13C 预设
        #[arg(long)]
        c13: bool,
    },
    /// 用内置合成数据跑一遍全流程（自检）
    Demo,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> nmrstuff_core::Result<()> {
    match cli.cmd {
        Cmd::Info { path } => {
            let loaded = nmrstuff_formats::load(&path)?;
            println!("文件：{path}");
            println!("来源：{}", loaded.source.vendor.label());
            println!("标题：{}", loaded.title);
            println!("核种：{}", loaded.nucleus().display());
            match &loaded.data {
                LoadedData::Fid(f) => {
                    println!("类型：时域 FID");
                    println!("点数：{}", f.len());
                    println!("观测频率：{:.4} MHz", f.meta.spectrometer_mhz);
                    println!("谱宽：{:.1} Hz", f.meta.spectral_width_hz);
                    println!("载波：{:.3} ppm", f.meta.carrier_ppm);
                    if let Some(s) = &f.meta.solvent {
                        println!("溶剂：{s}");
                    }
                    if let Some(g) = f.meta.group_delay {
                        println!("群延迟：{g:.2} 点");
                    }
                }
                LoadedData::Spectrum(s) => {
                    println!("类型：频域谱");
                    println!("点数：{}", s.y.len());
                    println!("范围：{:.3} – {:.3} ppm", s.x.start_ppm, s.x.end_ppm);
                }
            }
            for n in &loaded.notes {
                println!("注：{n}");
            }
        }
        Cmd::Proc { path, out, c13 } => {
            let loaded = nmrstuff_formats::load(&path)?;
            let steps = if c13 {
                nmrstuff_core::proc_op::recommended_13c()
            } else {
                nmrstuff_core::proc_op::recommended_1h()
            };
            let recipe = ProcRecipe::new(steps);
            let spec = match loaded.data {
                LoadedData::Fid(f) => nmrstuff_proc::process(&f, &recipe.steps)?.spectrum,
                LoadedData::Spectrum(s) => s,
            };
            let peaks = nmrstuff_analysis::pick(&spec, &PickParams::default());
            let multiplets = multiplets_from_auto(&spec, &peaks, &Default::default());
            let analysis = Analysis {
                peaks,
                multiplets,
                ..Default::default()
            };

            let csv = nmrstuff_report::peak_table_csv(&analysis);
            match out {
                Some(p) => {
                    std::fs::write(&p, csv)?;
                    println!("已写入 {}", p.display());
                }
                None => print!("{csv}"),
            }
            println!("处理链：");
            for l in recipe.describe_lines() {
                println!("  - {l}");
            }
        }
        Cmd::Demo => {
            let fid = nmrstuff_formats::synthetic::demo_ethyl_acetate();
            let recipe = ProcRecipe::default();
            let output = nmrstuff_proc::process(&fid, &recipe.steps)?;
            let spec = &output.spectrum;
            println!(
                "处理完成：{} 点，耗时 {:.1} ms",
                spec.y.len(),
                output.report.elapsed_ms
            );
            for w in &output.report.warnings {
                println!("警告：{w}");
            }
            let peaks = nmrstuff_analysis::pick(spec, &PickParams::default());
            println!("检出 {} 个峰", peaks.len());
            for p in peaks.iter().take(12) {
                println!("  δ {:.3}  高 {:.2}", p.ppm, p.height);
            }
            let multiplets = nmrstuff_analysis::multiplets_from_auto(
                spec,
                &nmrstuff_analysis::pick(spec, &PickParams::default()),
                &Default::default(),
            );
            println!("自动分组多重峰 {} 组：", multiplets.len());
            for m in &multiplets {
                println!(
                    "  {}  J = {} Hz",
                    m.range_text(2),
                    m.j_text(1).unwrap_or_else(|| "—".into())
                );
            }
            let peaks = nmrstuff_analysis::pick(spec, &PickParams::default());
            let mut integrals: Vec<nmrstuff_core::IntegralRegion> = multiplets
                .iter()
                .map(|m| {
                    {
                        let pad = ((m.from_ppm - m.to_ppm).abs() * 0.35).max(0.03);
                        nmrstuff_core::IntegralRegion::new(
                            m.from_ppm + pad,
                            m.to_ppm - pad,
                            nmrstuff_analysis::region_sum(spec, m.from_ppm + pad, m.to_ppm - pad),
                        )
                    }
                })
                .collect();
            let scale = nmrstuff_analysis::assign_by_area(&mut integrals, &[1.0, 2.0, 3.0, 4.0]);
            println!("\n积分（按面积归一，缩放因子 {scale:.0}）：");
            for r in &integrals {
                let h = if (r.value - r.value.round()).abs() < 1e-3 {
                    format!("{:.0}H", r.value)
                } else {
                    format!("{:.1}H", r.value)
                };
                println!("  {:.3}–{:.3}  {}", r.from_ppm, r.to_ppm, h);
            }
            let analysis = Analysis {
                peaks,
                multiplets,
                integrals,
            };
            println!("\nJACS 数据行：\n{}", nmrstuff_report::data_line(&analysis, nmrstuff_report::JournalStyle::Jacs));
        }
    }
    Ok(())
}
