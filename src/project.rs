use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectMode {
    #[default]
    Carousel,
    Float,
    Spin,
}

impl EffectMode {
    pub const ALL: [Self; 3] = [Self::Carousel, Self::Float, Self::Spin];

    pub fn label(self) -> &'static str {
        match self {
            Self::Carousel => "透明輪播",
            Self::Float => "漂浮循環",
            Self::Spin => "旋轉循環",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Carousel => "依序切換圖片，最後一張會平滑接回第一張。",
            Self::Float => "單張圖片沿著柔和的橢圓路徑來回漂浮。",
            Self::Spin => "單張圖片完整旋轉一圈，並保留透明邊界。",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputFormat {
    #[default]
    ProRes,
    Qtrle,
    Webm,
}

impl OutputFormat {
    pub const ALL: [Self; 3] = [Self::ProRes, Self::Qtrle, Self::Webm];

    pub fn label(self) -> &'static str {
        match self {
            Self::ProRes => "ProRes 4444 MOV",
            Self::Qtrle => "QTRLE MOV（無損）",
            Self::Webm => "VP9 WebM",
        }
    }

    pub fn encoder(self) -> &'static str {
        match self {
            Self::ProRes => "prores_ks",
            Self::Qtrle => "qtrle",
            Self::Webm => "libvpx-vp9",
        }
    }

    pub fn filename(self) -> &'static str {
        match self {
            Self::ProRes => "overlay_prores.mov",
            Self::Qtrle => "overlay_qtrle.mov",
            Self::Webm => "overlay_webm.webm",
        }
    }

    pub fn is_webm(self) -> bool {
        matches!(self, Self::Webm)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub images: Vec<PathBuf>,
    pub mode: EffectMode,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub seconds_per_image: f32,
    pub transition_seconds: f32,
    pub format: OutputFormat,
    pub keep_frames: bool,
    pub make_preview: bool,
    pub output_dir: PathBuf,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            images: Vec::new(),
            mode: EffectMode::Carousel,
            width: 960,
            height: 540,
            fps: 30,
            seconds_per_image: 3.0,
            transition_seconds: 1.0,
            format: OutputFormat::ProRes,
            keep_frames: false,
            make_preview: true,
            output_dir: PathBuf::from("output"),
        }
    }
}

impl Project {
    pub fn slot_frames(&self) -> Result<usize> {
        let raw = self.seconds_per_image * self.fps as f32;
        if !raw.is_finite() || raw < 2.0 || (raw - raw.round()).abs() > 0.0001 {
            bail!("每張圖片的秒數 × fps 必須是至少 2 的整數幀。");
        }
        Ok(raw.round() as usize)
    }

    pub fn transition_frames(&self) -> Result<usize> {
        if self.mode != EffectMode::Carousel {
            return Ok(0);
        }
        let raw = self.transition_seconds * self.fps as f32;
        if !raw.is_finite() || raw < 0.0 || (raw - raw.round()).abs() > 0.0001 {
            bail!("轉場秒數 × fps 必須是整數幀。");
        }
        Ok(raw.round() as usize)
    }

    pub fn total_frames(&self) -> Result<usize> {
        let slots = if self.mode == EffectMode::Carousel {
            self.images.len()
        } else {
            1
        };
        self.slot_frames()?
            .checked_mul(slots)
            .context("動畫影格數超過可處理範圍")
    }

    pub fn validate(&self) -> Result<()> {
        if self.images.is_empty() {
            bail!("請先加入至少一張透明圖片。");
        }
        if self.width < 64 || self.height < 64 || self.width % 2 != 0 || self.height % 2 != 0 {
            bail!("畫布寬高必須是 64 以上的偶數。");
        }
        if !(1..=120).contains(&self.fps) {
            bail!("fps 必須介於 1 到 120。");
        }

        let slot = self.slot_frames()?;
        if self.mode == EffectMode::Carousel {
            let transition = self.transition_frames()?;
            if transition == 0 || transition >= slot {
                bail!("輪播轉場至少 1 幀，且需短於每張圖片的時段。");
            }
        }

        if self.output_dir.exists() {
            if !self.output_dir.is_dir() {
                bail!("輸出位置不是資料夾，請重新選擇。");
            }
            if std::fs::read_dir(&self.output_dir)?.next().is_some() {
                bail!("輸出資料夾不是空的；請選擇新資料夾以保護既有檔案。");
            }
        }
        for path in &self.images {
            if !path.is_file() {
                bail!("找不到圖片：{}", path.display());
            }
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json).with_context(|| format!("無法寫入專案：{}", path.display()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("無法讀取專案：{}", path.display()))?;
        serde_json::from_str(&data).context("專案 JSON 格式有誤")
    }
}
