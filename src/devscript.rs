//! Scripted UI automation for development & visual regression checks.
//!
//! `TRUNQ_SCRIPT="wait:2;run;wait:2;shot:/tmp/a.png;quit"` drives the app
//! without any OS-level accessibility / screen-recording permissions.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static PENDING_SHOTS: AtomicUsize = AtomicUsize::new(0);

pub enum Step {
    Wait(f32),
    Shot(String),
    Run,
    DryRun,
    RunAll,
    Explain(bool),
    OpenTable(String, String),
    Sql(String),
    Tab(usize),
    Theme(bool),
    NewConnection(String),
    Extensions(String),
    CloseDialogs,
    Connections,
    Settings,
    History,
    Palette(String),
    Health,
    Grants,
    Cron,
    Chart,
    /// Query-tab level helpers: card, compare, value:r,c, pin, side, search:x, ts:utc
    TabDev(String, String),
    Sessions,
    Structure(String),
    Erd(String),
    Resize(f32, f32),
    Import(String),
    ImportRun,
    Quit,
}

pub struct Script {
    steps: VecDeque<Step>,
    next_at: Instant,
}

impl Script {
    pub fn from_env() -> Option<Script> {
        let raw = std::env::var("TRUNQ_SCRIPT").ok()?;
        let steps = raw
            .split(";;")
            .flat_map(|chunk| if chunk.starts_with("sql:") { vec![chunk.to_string()] } else { chunk.split(';').map(str::to_string).collect() })
            .filter_map(|s| {
                let s = s.trim();
                let (cmd, arg) = s.split_once(':').unwrap_or((s, ""));
                Some(match cmd {
                    "wait" => Step::Wait(arg.parse().unwrap_or(1.0)),
                    "shot" => Step::Shot(arg.to_string()),
                    "run" => Step::Run,
                    "dryrun" => Step::DryRun,
                    "runall" => Step::RunAll,
                    "explain" => Step::Explain(arg == "analyze"),
                    "open" => {
                        let (s, n) = arg.split_once('.').unwrap_or(("public", arg));
                        Step::OpenTable(s.to_string(), n.to_string())
                    }
                    "sql" => Step::Sql(arg.replace("\\n", "\n")),
                    "tab" => Step::Tab(arg.parse().unwrap_or(0)),
                    "theme" => Step::Theme(arg == "light"),
                    "newconn" => Step::NewConnection(arg.to_string()),
                    "extensions" => Step::Extensions(arg.to_string()),
                    "close" => Step::CloseDialogs,
                    "connections" => Step::Connections,
                    "settings" => Step::Settings,
                    "history" => Step::History,
                    "palette" => Step::Palette(arg.to_string()),
                    "health" => Step::Health,
                    "grants" => Step::Grants,
                    "cron" => Step::Cron,
                    "chart" => Step::Chart,
                    "card" | "compare" | "value" | "pin" | "side" | "search" | "ts" | "finder" | "hideempty" | "inspect" | "bottom" | "stoploading" => {
                        Step::TabDev(cmd.to_string(), arg.to_string())
                    }
                    "sessions" => Step::Sessions,
                    "structure" => Step::Structure(arg.to_string()),
                    "erd" => Step::Erd(arg.to_string()),
                    "resize" => {
                        let (w, h) = arg.split_once(',')?;
                        Step::Resize(w.parse().ok()?, h.parse().ok()?)
                    }
                    "import" => Step::Import(arg.to_string()),
                    "importrun" => Step::ImportRun,
                    "quit" => Step::Quit,
                    "" => return None,
                    other => {
                        eprintln!("TRUNQ_SCRIPT: unknown step {other}");
                        return None;
                    }
                })
            })
            .collect();
        Some(Script { steps, next_at: Instant::now() + Duration::from_millis(800) })
    }

    /// Returns the next step if it is due.
    pub fn next(&mut self) -> Option<Step> {
        if Instant::now() < self.next_at || PENDING_SHOTS.load(Ordering::Acquire) != 0 {
            return None;
        }
        let step = self.steps.pop_front()?;
        self.next_at = Instant::now() + Duration::from_millis(300);
        if let Step::Wait(s) = step {
            self.next_at = Instant::now() + Duration::from_secs_f32(s);
            return None;
        }
        Some(step)
    }

    pub fn done(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn waiting_for_screenshot(&self) -> bool {
        PENDING_SHOTS.load(Ordering::Acquire) != 0
    }
}

pub fn screenshot(ctx: &egui::Context, path: String) {
    PENDING_SHOTS.fetch_add(1, Ordering::AcqRel);
    let repaint = ctx.clone();
    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
        // Screenshot callbacks run while the GPU queue is submitted. Encoding
        // PNG there stalls every frame, especially with an unoptimised build.
        std::thread::spawn(move || {
            let saved = (|| -> anyhow::Result<()> {
                use image::ImageEncoder as _;
                use std::io::Write as _;
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
                image::codecs::png::PngEncoder::new_with_quality(&mut file, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Sub)
                    .write_image(&bytes, img.size[0] as u32, img.size[1] as u32, image::ExtendedColorType::Rgba8)?;
                file.flush()?;
                Ok(())
            })();
            match saved {
                Ok(()) => eprintln!("TRUNQ_SCRIPT: saved {path}"),
                Err(e) => eprintln!("TRUNQ_SCRIPT: could not save {path}: {e}"),
            }
            PENDING_SHOTS.fetch_sub(1, Ordering::AcqRel);
            repaint.request_repaint();
        });
    })));
}
