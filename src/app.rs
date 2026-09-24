use std::{
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, unbounded};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Frame, Layout, RichText, ScrollArea, Stroke,
    TextureHandle, TextureOptions, Vec2,
};
use image::{Rgba, RgbaImage};

use crate::{
    core, ffmpeg,
    project::{EffectMode, OutputFormat, Project},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Workspace {
    Cards,
    Overlay,
}

#[derive(Clone)]
enum JobMessage {
    Progress(f32, String),
    Finished(String, Option<PathBuf>),
    Failed(String),
}

struct OverlayDraft {
    base_video: PathBuf,
    alpha_asset: PathBuf,
    output_video: PathBuf,
    x: i32,
    y: i32,
    width: String,
    start: f64,
    end: String,
    use_source_fps: bool,
    fps: String,
}

impl Default for OverlayDraft {
    fn default() -> Self {
        Self {
            base_video: PathBuf::new(),
            alpha_asset: PathBuf::new(),
            output_video: PathBuf::new(),
            x: 0,
            y: 0,
            width: String::new(),
            start: 0.0,
            end: String::new(),
            use_source_fps: true,
            fps: "30".to_owned(),
        }
    }
}

pub struct StudioApp {
    project: Project,
    workspace: Workspace,
    overlay: OverlayDraft,
    selected_image: Option<usize>,
    preview_frame: usize,
    preview_playing: bool,
    preview_last_tick: Instant,
    preview_texture: Option<TextureHandle>,
    cached_assets: Option<Vec<RgbaImage>>,
    assets_dirty: bool,
    preview_dirty: bool,
    keep_project_path: Option<PathBuf>,
    busy: bool,
    progress: f32,
    status: String,
    error: Option<String>,
    notice: Option<String>,
    receiver: Option<Receiver<JobMessage>>,
}

impl StudioApp {
    pub fn new(context: &eframe::CreationContext<'_>) -> Self {
        let has_cjk_font = install_traditional_chinese_font(&context.egui_ctx);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(13, 18, 26);
        visuals.window_fill = Color32::from_rgb(19, 26, 36);
        visuals.extreme_bg_color = Color32::from_rgb(9, 13, 19);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(29, 39, 52);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(43, 60, 77);
        visuals.widgets.active.bg_fill = Color32::from_rgb(50, 177, 162);
        visuals.selection.bg_fill = Color32::from_rgb(28, 133, 129);
        visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        visuals.override_text_color = Some(Color32::from_rgb(231, 238, 244));
        context.egui_ctx.set_visuals(visuals);

        Self {
            project: Project::default(),
            workspace: Workspace::Cards,
            overlay: OverlayDraft::default(),
            selected_image: None,
            preview_frame: 0,
            preview_playing: false,
            preview_last_tick: Instant::now(),
            preview_texture: None,
            cached_assets: None,
            assets_dirty: true,
            preview_dirty: true,
            keep_project_path: None,
            busy: false,
            progress: 0.0,
            status: if has_cjk_font {
                "加入透明 PNG，選擇轉場效果後即可輸出。".to_owned()
            } else {
                "找不到繁體中文字型，請安裝 Microsoft JhengHei 或 Noto Sans TC。".to_owned()
            },
            error: None,
            notice: None,
            receiver: None,
        }
    }

    fn set_dirty(&mut self) {
        self.preview_dirty = true;
    }

    fn tick_preview(&mut self) {
        if !self.preview_playing {
            return;
        }
        let fps = self.project.fps.max(1) as f64;
        let frame_time = Duration::from_secs_f64(1.0 / fps);
        let now = Instant::now();
        let elapsed = now.duration_since(self.preview_last_tick);
        let frames = (elapsed.as_secs_f64() * fps).floor() as usize;
        if frames > 0 {
            let total = self.project.total_frames().unwrap_or(1).max(1);
            self.preview_frame = (self.preview_frame + frames) % total;
            self.preview_last_tick += frame_time * frames.min(u32::MAX as usize) as u32;
            self.set_dirty();
        }
    }

    fn set_assets_dirty(&mut self) {
        self.preview_playing = false;
        self.assets_dirty = true;
        self.cached_assets = None;
        self.preview_dirty = true;
    }

    fn pick_images(&mut self) {
        let paths = rfd::FileDialog::new()
            .add_filter("圖片", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
            .pick_files();
        if let Some(paths) = paths {
            self.project.images.extend(paths);
            if self.selected_image.is_none() && !self.project.images.is_empty() {
                self.selected_image = Some(0);
            }
            self.preview_frame = 0;
            self.set_assets_dirty();
            self.notice = Some(format!("已加入 {} 張圖片。", self.project.images.len()));
        }
    }

    fn replace_selected_image(&mut self) {
        let Some(index) = self.selected_image else {
            self.error = Some("先在素材清單選取要替換的圖片。".to_owned());
            return;
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("圖片", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
            .pick_file()
        {
            if let Some(target) = self.project.images.get_mut(index) {
                *target = path;
                self.set_assets_dirty();
                self.notice = Some("已替換選取圖片，輸出設定已保留。".to_owned());
            }
        }
    }

    fn load_samples(&mut self) {
        let folder = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("samples")
            .join("source_images");
        let mut paths: Vec<_> = match std::fs::read_dir(&folder) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().is_some_and(|ext| {
                        ["png", "jpg", "jpeg", "webp", "bmp"]
                            .iter()
                            .any(|allowed| ext.eq_ignore_ascii_case(allowed))
                    })
                })
                .collect(),
            Err(error) => {
                self.error = Some(format!("找不到內附範例圖片：{error}"));
                return;
            }
        };
        paths.sort();
        if paths.is_empty() {
            self.error = Some("內附範例資料夾目前沒有圖片。".to_owned());
            return;
        }
        self.project.images = paths;
        self.selected_image = Some(0);
        self.workspace = Workspace::Cards;
        self.preview_frame = 0;
        self.set_assets_dirty();
        self.notice = Some("已載入 3 張範例素材，可直接預覽或輸出。".to_owned());
    }

    fn remove_selected_image(&mut self) {
        if let Some(index) = self.selected_image
            && index < self.project.images.len()
        {
            self.project.images.remove(index);
            self.selected_image = if self.project.images.is_empty() {
                None
            } else {
                Some(index.min(self.project.images.len() - 1))
            };
            self.preview_frame = 0;
            self.set_assets_dirty();
        }
    }

    fn move_selected_image(&mut self, delta: isize) {
        let Some(index) = self.selected_image else {
            return;
        };
        let target = index as isize + delta;
        if target < 0 || target >= self.project.images.len() as isize {
            return;
        }
        self.project.images.swap(index, target as usize);
        self.selected_image = Some(target as usize);
        self.set_assets_dirty();
    }

    fn save_project(&mut self) {
        let path = if let Some(path) = self.keep_project_path.clone() {
            path
        } else if let Some(path) = rfd::FileDialog::new()
            .add_filter("透明字卡專案", &["tcard.json"])
            .set_file_name("my_cards.tcard.json")
            .save_file()
        {
            self.keep_project_path = Some(path.clone());
            path
        } else {
            return;
        };
        match self.project.save(&path) {
            Ok(()) => self.notice = Some(format!("專案設定已儲存：{}", path.display())),
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    fn load_project(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("透明字卡專案", &["json"])
            .pick_file()
        else {
            return;
        };
        match Project::load(&path) {
            Ok(project) => {
                self.project = project;
                self.keep_project_path = Some(path);
                self.selected_image = (!self.project.images.is_empty()).then_some(0);
                self.preview_frame = 0;
                self.set_assets_dirty();
                self.notice = Some("專案設定已載入，可替換清單中的圖片。".to_owned());
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    fn start_render(&mut self) {
        match self.project.validate() {
            Ok(()) => {}
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                return;
            }
        }
        let project = self.project.clone();
        let (sender, receiver) = unbounded();
        self.receiver = Some(receiver);
        self.busy = true;
        self.preview_playing = false;
        self.progress = 0.0;
        self.status = "開始準備透明動畫…".to_owned();
        self.error = None;
        let sender_for_worker = sender.clone();
        thread::spawn(move || {
            let result = ffmpeg::render_master(&project, |fraction, message| {
                let _ = sender_for_worker.send(JobMessage::Progress(fraction, message));
            });
            match result {
                Ok(result) => {
                    let mut message = format!(
                        "透明影片：{}\n{} 幀 · {:.1} 秒",
                        result.master.display(),
                        result.frame_count,
                        result.duration_seconds
                    );
                    if let Some(preview) = result.preview {
                        message.push_str(&format!("\n棋盤格預覽：{}", preview.display()));
                    }
                    if !result.warnings.is_empty() {
                        message.push_str("\n提醒：");
                        message.push_str(&result.warnings.join("；"));
                    }
                    let _ = sender.send(JobMessage::Finished(message, Some(result.master)));
                }
                Err(error) => {
                    let _ = sender.send(JobMessage::Failed(format!("{error:#}")));
                }
            }
        });
    }

    fn start_overlay(&mut self) {
        let base = self.overlay.base_video.clone();
        let alpha = self.overlay.alpha_asset.clone();
        let output = self.overlay.output_video.clone();
        let x = self.overlay.x;
        let y = self.overlay.y;
        let width = if self.overlay.width.trim().is_empty() {
            None
        } else {
            match self.overlay.width.trim().parse::<u32>() {
                Ok(width) => Some(width),
                Err(_) => {
                    self.error = Some("寬度請填正偶數，或留空維持原尺寸。".to_owned());
                    return;
                }
            }
        };
        let end = if self.overlay.end.trim().is_empty() {
            None
        } else {
            match self.overlay.end.trim().parse::<f64>() {
                Ok(value) => Some(value),
                Err(_) => {
                    self.error = Some("結束時間請填秒數，或留空延伸到片尾。".to_owned());
                    return;
                }
            }
        };
        let start = self.overlay.start;
        let fps = (!self.overlay.use_source_fps).then(|| self.overlay.fps.trim().to_owned());
        let (sender, receiver) = unbounded();
        self.receiver = Some(receiver);
        self.busy = true;
        self.progress = 0.0;
        self.status = "開始合成主影片…".to_owned();
        self.error = None;
        thread::spawn(move || {
            let result = ffmpeg::overlay_on_video(
                &base,
                &alpha,
                &output,
                x,
                y,
                width,
                start,
                end,
                fps.as_deref(),
                |fraction, message| {
                    let _ = sender.send(JobMessage::Progress(fraction, message));
                },
            );
            match result {
                Ok(()) => {
                    let _ = sender.send(JobMessage::Finished(
                        format!("最終影片：{}", output.display()),
                        None,
                    ));
                }
                Err(error) => {
                    let _ = sender.send(JobMessage::Failed(format!("{error:#}")));
                }
            }
        });
    }

    fn poll_job(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        let mut finished = false;
        while let Ok(message) = receiver.try_recv() {
            match message {
                JobMessage::Progress(progress, status) => {
                    self.progress = progress.clamp(0.0, 1.0);
                    self.status = status;
                }
                JobMessage::Finished(message, generated_asset) => {
                    if let Some(path) = generated_asset {
                        self.overlay.alpha_asset = path;
                        self.workspace = Workspace::Cards;
                        self.preview_frame = 0;
                        self.preview_last_tick = Instant::now();
                        self.preview_playing = true;
                        self.cached_assets = None;
                        self.assets_dirty = true;
                        self.preview_dirty = true;
                    }
                    self.progress = 1.0;
                    self.status = "工作完成".to_owned();
                    self.notice = Some(message);
                    finished = true;
                }
                JobMessage::Failed(message) => {
                    self.status = "輸出失敗".to_owned();
                    self.error = Some(message);
                    finished = true;
                }
            }
        }
        if finished {
            self.busy = false;
            self.receiver = None;
        }
    }

    fn refresh_preview(&mut self, context: &egui::Context) {
        if !self.preview_dirty {
            return;
        }
        self.preview_dirty = false;
        if self.project.images.is_empty() {
            self.cached_assets = None;
            self.preview_texture = None;
            return;
        }

        if self.assets_dirty || self.cached_assets.is_none() {
            match core::load_assets(&self.project) {
                Ok((assets, warnings)) => {
                    self.cached_assets = Some(assets);
                    self.assets_dirty = false;
                    if !warnings.is_empty() {
                        self.status = warnings.join("；");
                    }
                }
                Err(error) => {
                    self.cached_assets = None;
                    self.preview_texture = None;
                    self.status = format!("預覽無法載入：{error:#}");
                    return;
                }
            }
        }

        let Some(assets) = &self.cached_assets else {
            return;
        };
        match core::render_frame(self.preview_frame, assets, &self.project) {
            Ok(frame) => {
                let checker = core::checkerboard(self.project.width, self.project.height);
                let flattened = flatten_to_background(&frame, &checker);
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [flattened.width() as usize, flattened.height() as usize],
                    flattened.as_raw(),
                );
                if let Some(texture) = &mut self.preview_texture {
                    texture.set(color_image, TextureOptions::LINEAR);
                } else {
                    self.preview_texture = Some(context.load_texture(
                        "transparent-card-preview",
                        color_image,
                        TextureOptions::LINEAR,
                    ));
                }
            }
            Err(error) => {
                self.status = format!("預覽產生失敗：{error:#}");
                self.preview_texture = None;
            }
        }
    }

    fn handle_dropped_files(&mut self, context: &egui::Context) {
        let dropped = context.input(|input| input.raw.dropped_files.clone());
        let paths: Vec<_> = dropped
            .into_iter()
            .filter_map(|file| file.path)
            .filter(|path| supported_image(path))
            .collect();
        if paths.is_empty() {
            return;
        }
        self.project.images.extend(paths.iter().cloned());
        if self.selected_image.is_none() {
            self.selected_image = Some(0);
        }
        self.set_assets_dirty();
        self.notice = Some(format!("已從桌面加入 {} 張圖片。", paths.len()));
    }

    fn top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("header")
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(16, 23, 32))
                    .inner_margin(egui::Margin::symmetric(24, 14))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(34, 47, 62))),
            )
            .show_inside(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("◈")
                            .color(Color32::from_rgb(74, 220, 194))
                            .size(30.0),
                    );
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("透明字卡工作室")
                                .font(FontId::proportional(21.0))
                                .strong(),
                        );
                        ui.label(
                            RichText::new("動態透明素材 · 快速替換 · 直接輸出")
                                .color(Color32::from_rgb(142, 160, 179))
                                .size(12.5),
                        );
                    });
                    ui.add_space(26.0);
                    self.nav_button(ui, Workspace::Cards, "01  字卡製作");
                    self.nav_button(ui, Workspace::Overlay, "02  疊入主影片");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.busy, secondary_button("載入專案"))
                            .clicked()
                        {
                            self.load_project();
                        }
                        if ui
                            .add_enabled(!self.busy, secondary_button("儲存專案"))
                            .clicked()
                        {
                            self.save_project();
                        }
                    });
                });
            });
    }

    fn nav_button(&mut self, ui: &mut egui::Ui, workspace: Workspace, label: &str) {
        let active = self.workspace == workspace;
        let button = egui::Button::new(RichText::new(label).color(if active {
            Color32::from_rgb(103, 230, 207)
        } else {
            Color32::from_rgb(166, 181, 198)
        }))
        .fill(if active {
            Color32::from_rgb(28, 68, 71)
        } else {
            Color32::TRANSPARENT
        })
        .stroke(Stroke::new(
            1.0,
            if active {
                Color32::from_rgb(44, 118, 112)
            } else {
                Color32::TRANSPARENT
            },
        ))
        .corner_radius(egui::CornerRadius::same(9));
        if ui.add(button).clicked() {
            self.workspace = workspace;
        }
    }

    fn cards_workspace(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        ui.columns(2, |columns| {
            self.cards_controls(&mut columns[0], context);
            self.preview_panel(&mut columns[1], context);
        });
    }

    fn cards_controls(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        ScrollArea::vertical()
            .id_salt("cards-controls-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.section(ui, "素材清單", "快速加入、替換與調整輪播順序");
                ui.label(
                    RichText::new(format!(
                        "輸入規格：PNG／WebP 透明背景最佳；也支援 JPG、BMP、GIF 首格。\n原圖建議至少 {} × {} px；會保持比例縮放並置中。白底不會自動去背。",
                        (self.project.width as f32 * 0.64).floor() as u32,
                        (self.project.height as f32 * 0.64).floor() as u32,
                    ))
                    .color(Color32::from_rgb(153, 181, 196))
                    .size(11.5),
                );
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.busy, primary_button("＋ 加入圖片"))
                        .clicked()
                    {
                        self.pick_images();
                    }
                    if ui
                        .add_enabled(!self.busy, secondary_button("載入範例"))
                        .clicked()
                    {
                        self.load_samples();
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.busy && self.selected_image.is_some(),
                            secondary_button("替換"),
                        )
                        .clicked()
                    {
                        self.replace_selected_image();
                    }
                    if ui
                        .add_enabled(
                            !self.busy && self.selected_image.is_some(),
                            secondary_button("↑"),
                        )
                        .clicked()
                    {
                        self.move_selected_image(-1);
                    }
                    if ui
                        .add_enabled(
                            !self.busy && self.selected_image.is_some(),
                            secondary_button("↓"),
                        )
                        .clicked()
                    {
                        self.move_selected_image(1);
                    }
                    if ui
                        .add_enabled(
                            !self.busy && self.selected_image.is_some(),
                            secondary_button("移除"),
                        )
                        .clicked()
                    {
                        self.remove_selected_image();
                    }
                });

                Frame::new()
                    .fill(Color32::from_rgb(16, 24, 34))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(34, 48, 63)))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        if self.project.images.is_empty() {
                            ui.add_space(18.0);
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    RichText::new("將圖片拖進這裡，或按「加入圖片」")
                                        .color(Color32::from_rgb(137, 157, 178)),
                                );
                            });
                            ui.add_space(18.0);
                        } else {
                            ScrollArea::vertical()
                                .id_salt("image-list")
                                .max_height(172.0)
                                .show(ui, |ui| {
                                    for (index, path) in self.project.images.iter().enumerate() {
                                        let name =
                                            path.file_name().unwrap_or_default().to_string_lossy();
                                        let selected = self.selected_image == Some(index);
                                        let text = format!("{:02}  {}", index + 1, name);
                                        let response = ui.selectable_label(
                                            selected,
                                            RichText::new(text).color(if selected {
                                                Color32::from_rgb(183, 250, 230)
                                            } else {
                                                Color32::from_rgb(194, 204, 216)
                                            }),
                                        );
                                        if response.clicked() {
                                            self.selected_image = Some(index);
                                        }
                                    }
                                });
                        }
                    });

                ui.add_space(16.0);
                self.section(ui, "切換效果", "每種效果都會保留透明背景");
                ui.horizontal_wrapped(|ui| {
                    for mode in EffectMode::ALL {
                        let selected = self.project.mode == mode;
                        let button =
                            egui::Button::new(RichText::new(mode.label()).color(if selected {
                                Color32::from_rgb(111, 238, 211)
                            } else {
                                Color32::from_rgb(181, 195, 209)
                            }))
                            .fill(if selected {
                                Color32::from_rgb(29, 72, 73)
                            } else {
                                Color32::from_rgb(24, 34, 46)
                            })
                            .corner_radius(egui::CornerRadius::same(8));
                        if ui.add(button).clicked() {
                            self.project.mode = mode;
                            self.preview_frame = 0;
                            self.set_assets_dirty();
                        }
                    }
                });
                ui.label(
                    RichText::new(self.project.mode.hint())
                        .color(Color32::from_rgb(143, 162, 181))
                        .size(12.0),
                );
                if self.project.mode != EffectMode::Carousel && self.project.images.len() > 1 {
                    ui.label(
                        RichText::new("單圖效果使用清單第一張；使用 ↑ ↓ 調整主圖。")
                            .color(Color32::from_rgb(242, 196, 112))
                            .size(11.5),
                    );
                }

                ui.add_space(15.0);
                self.section(ui, "輸出設定", "畫布與影格率會套用到整段動畫");
                egui::Grid::new("canvas-settings")
                    .num_columns(4)
                    .spacing([8.0, 7.0])
                    .show(ui, |ui| {
                        ui.label("寬");
                        let width = ui.add(
                            egui::DragValue::new(&mut self.project.width)
                                .range(64..=7680)
                                .speed(16.0)
                                .suffix(" px"),
                        );
                        ui.label("高");
                        let height = ui.add(
                            egui::DragValue::new(&mut self.project.height)
                                .range(64..=4320)
                                .speed(16.0)
                                .suffix(" px"),
                        );
                        if width.changed() || height.changed() {
                            self.set_assets_dirty();
                        }
                        ui.end_row();

                        ui.label("影格率");
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.project.fps)
                                    .range(1..=120)
                                    .suffix(" fps"),
                            )
                            .changed()
                        {
                            self.preview_frame = 0;
                            self.set_dirty();
                        }
                        ui.label("每張");
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.project.seconds_per_image)
                                    .range(0.1..=60.0)
                                    .speed(0.1)
                                    .fixed_decimals(1)
                                    .suffix(" 秒"),
                            )
                            .changed()
                        {
                            self.preview_frame = 0;
                            self.set_dirty();
                        }
                        ui.end_row();

                        ui.label("轉場");
                        let transition = ui.add(
                            egui::DragValue::new(&mut self.project.transition_seconds)
                                .range(0.0..=30.0)
                                .speed(0.1)
                                .fixed_decimals(1)
                                .suffix(" 秒"),
                        );
                        ui.label("格式");
                        egui::ComboBox::from_id_salt("output-format")
                            .selected_text(self.project.format.label())
                            .width(132.0)
                            .show_ui(ui, |ui| {
                                for format in OutputFormat::ALL {
                                    ui.selectable_value(
                                        &mut self.project.format,
                                        format,
                                        format.label(),
                                    );
                                }
                            });
                        if transition.changed() {
                            self.set_dirty();
                        }
                        ui.end_row();
                    });

                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.project.make_preview, "同時輸出棋盤格 MP4 預覽");
                    ui.checkbox(&mut self.project.keep_frames, "保留 PNG 逐格素材");
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("輸出位置：{}", self.project.output_dir.display()))
                            .color(Color32::from_rgb(147, 168, 187))
                            .size(11.5),
                    );
                    if ui.add(secondary_button("選擇")).clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        self.project.output_dir = path;
                    }
                });
                ui.add_space(8.0);
                if ui
                    .add_enabled(
                        !self.busy && !self.project.images.is_empty(),
                        primary_button("輸出透明動畫"),
                    )
                    .clicked()
                {
                    self.start_render();
                }
            });
        let _ = context;
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        self.refresh_preview(context);
        ui.vertical(|ui| {
            Frame::new()
                .fill(Color32::from_rgb(17, 25, 35))
                .stroke(Stroke::new(1.0, Color32::from_rgb(39, 54, 72)))
                .corner_radius(egui::CornerRadius::same(14))
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("即時預覽").size(17.0).strong());
                        ui.label(
                            RichText::new("棋盤格背景僅供檢查")
                                .color(Color32::from_rgb(137, 157, 178))
                                .size(11.5),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} × {}",
                                    self.project.width, self.project.height
                                ))
                                .color(Color32::from_rgb(129, 201, 188))
                                .size(11.5),
                            );
                        });
                    });
                    ui.add_space(10.0);
                    let area = ui.available_size();
                    let target = fit_size(
                        Vec2::new(self.project.width as f32, self.project.height as f32),
                        Vec2::new(area.x.max(100.0), (area.y - 95.0).max(150.0)),
                    );
                    let (rect, _) = ui.allocate_exact_size(target, egui::Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        egui::CornerRadius::same(8),
                        Color32::from_rgb(54, 61, 73),
                    );
                    if let Some(texture) = &self.preview_texture {
                        ui.painter().image(
                            texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    } else {
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "加入素材以查看預覽",
                            FontId::proportional(15.0),
                            Color32::from_rgb(154, 173, 192),
                        );
                    }
                    ui.add_space(12.0);
                    let max_frame = self
                        .project
                        .total_frames()
                        .unwrap_or(1)
                        .saturating_sub(1)
                        .min(u32::MAX as usize) as u32;
                    let slider = ui.add(
                        egui::Slider::new(&mut self.preview_frame, 0..=max_frame as usize)
                            .text("預覽影格")
                            .show_value(false),
                    );
                    if slider.changed() {
                        self.preview_playing = false;
                        self.set_dirty();
                    }
                    ui.horizontal(|ui| {
                        let play_label = if self.preview_playing {
                            "暫停"
                        } else {
                            "播放"
                        };
                        if ui
                            .add_enabled(
                                !self.project.images.is_empty(),
                                secondary_button(play_label),
                            )
                            .clicked()
                        {
                            self.preview_playing = !self.preview_playing;
                            self.preview_last_tick = Instant::now();
                        }
                        let total = max_frame as usize + 1;
                        ui.label(
                            RichText::new(format!("第 {} / {} 幀", self.preview_frame + 1, total))
                                .color(Color32::from_rgb(149, 167, 185))
                                .size(11.5),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("重新整理預覽").clicked() {
                                self.set_assets_dirty();
                            }
                        });
                    });
                });

            ui.add_space(12.0);
            self.section(ui, "輸出說明", "保持透明素材，方便放入剪輯時間軸");
            ui.label(
                RichText::new(format!(
                    "{} 幀 · {:.1} 秒循環 · {}",
                    self.project.total_frames().unwrap_or(0),
                    self.project.total_frames().unwrap_or(0) as f32
                        / self.project.fps.max(1) as f32,
                    self.project.format.label()
                ))
                .color(Color32::from_rgb(153, 172, 190)),
            );
            ui.label(
                RichText::new("素材輸出採用直通 Alpha；白底圖片不會自動去背。")
                    .color(Color32::from_rgb(140, 158, 177))
                    .size(11.5),
            );
        });
    }

    fn overlay_workspace(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |columns| {
            let left = &mut columns[0];
            self.section(
                left,
                "主影片與透明素材",
                "透明動畫會循環播放，主片聲音會保留",
            );
            Self::file_picker_row(left, "主影片", &mut self.overlay.base_video, true);
            Self::file_picker_row(left, "透明素材", &mut self.overlay.alpha_asset, false);
            Self::file_picker_row(left, "輸出 MP4", &mut self.overlay.output_video, false);
            left.add_space(18.0);
            self.section(left, "位置與時間", "座標從主影片左上角開始計算");
            egui::Grid::new("overlay-settings")
                .num_columns(2)
                .spacing([12.0, 10.0])
                .show(left, |ui| {
                    ui.label("X");
                    ui.add(egui::DragValue::new(&mut self.overlay.x).speed(2));
                    ui.end_row();
                    ui.label("Y");
                    ui.add(egui::DragValue::new(&mut self.overlay.y).speed(2));
                    ui.end_row();
                    ui.label("素材寬度");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.overlay.width)
                                .desired_width(92.0)
                                .hint_text("留空不縮放"),
                        );
                        ui.label("px，偶數");
                    });
                    ui.end_row();
                    ui.label("開始");
                    ui.add(
                        egui::DragValue::new(&mut self.overlay.start)
                            .speed(0.1)
                            .fixed_decimals(1)
                            .suffix(" 秒"),
                    );
                    ui.end_row();
                    ui.label("結束");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.overlay.end)
                            .desired_width(100.0)
                            .hint_text("留空到片尾"),
                    );
                    ui.end_row();
                });
            left.add_space(12.0);
            left.checkbox(&mut self.overlay.use_source_fps, "沿用主影片影格率");
            if !self.overlay.use_source_fps {
                left.horizontal(|ui| {
                    ui.label("輸出 fps");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.overlay.fps)
                            .desired_width(105.0)
                            .hint_text("30 或 30000/1001"),
                    );
                });
            }
            left.add_space(14.0);
            if left
                .add_enabled(!self.busy, primary_button("輸出合成 MP4"))
                .clicked()
            {
                self.start_overlay();
            }
            let right = &mut columns[1];
            self.section(right, "合成資訊", "輸出一般影片格式，透明區會顯示主片");
            right.label("透明素材依起始時間開始播放，主片結束時停止。");
            right.label("可指定位置、寬度與顯示時間區段。");
            right.label("主片音訊會保留並轉成 AAC。");
            right.add_space(15.0);
            Frame::new()
                .fill(Color32::from_rgb(18, 27, 37))
                .stroke(Stroke::new(1.0, Color32::from_rgb(40, 56, 72)))
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(16.0)
                .show(right, |ui| {
                    ui.label(RichText::new("FFmpeg 工作流程").strong());
                    ui.add_space(8.0);
                    ui.label("• 支援 ProRes、QTRLE 與 VP9 透明素材");
                    ui.label("• 依主片影格率輸出 CFR MP4");
                    ui.label("• 影片尺寸為奇數時會補齊到偶數");
                });
        });
    }

    fn file_picker_row(ui: &mut egui::Ui, label: &str, target: &mut PathBuf, video: bool) {
        let current = target.display().to_string();
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).strong());
            ui.label(
                RichText::new(if current == "" {
                    "尚未選擇"
                } else {
                    &current
                })
                .color(Color32::from_rgb(143, 164, 182))
                .size(11.5),
            );
            if ui.add(secondary_button("瀏覽")).clicked() {
                let mut dialog = rfd::FileDialog::new();
                if video {
                    dialog = dialog.add_filter("影片", &["mp4", "mov", "mkv", "webm", "avi"]);
                } else if label == "透明素材" {
                    dialog = dialog.add_filter("透明影片", &["mov", "webm"]);
                }
                if video || label == "透明素材" {
                    if let Some(path) = dialog.pick_file() {
                        *target = path;
                    }
                } else if let Some(path) = dialog
                    .add_filter("MP4 影片", &["mp4"])
                    .set_file_name("final_video.mp4")
                    .save_file()
                {
                    *target = path;
                }
            }
        });
    }

    fn section(&self, ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(15.0).strong());
            ui.label(
                RichText::new(subtitle)
                    .color(Color32::from_rgb(134, 154, 174))
                    .size(11.5),
            );
        });
        ui.add_space(8.0);
    }

    fn bottom_status(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(15, 21, 30))
                    .inner_margin(egui::Margin::symmetric(22, 10))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(34, 47, 62))),
            )
            .show_inside(root, |ui| {
                if self.busy {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::ProgressBar::new(self.progress)
                                .desired_width(240.0)
                                .show_percentage(),
                        );
                        ui.label(&self.status);
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("●")
                                .color(Color32::from_rgb(72, 208, 173))
                                .size(10.0),
                        );
                        ui.label(
                            RichText::new(&self.status).color(Color32::from_rgb(148, 169, 189)),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new("Rust · FFmpeg")
                                    .color(Color32::from_rgb(104, 124, 144))
                                    .size(11.0),
                            );
                        });
                    });
                }
            });
    }

    fn popup_messages(&mut self, context: &egui::Context) {
        if let Some(message) = self.error.clone() {
            egui::Window::new("需要處理")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(context, |ui| {
                    ui.set_max_width(560.0);
                    ui.label(RichText::new(message).color(Color32::from_rgb(255, 173, 157)));
                    ui.add_space(12.0);
                    if ui.button("關閉").clicked() {
                        self.error = None;
                    }
                });
        }
        if let Some(message) = self.notice.clone() {
            egui::Window::new("已完成")
                .collapsible(false)
                .resizable(true)
                .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-25.0, -60.0))
                .show(context, |ui| {
                    ui.set_max_width(620.0);
                    ui.label(message);
                    ui.add_space(7.0);
                    if ui.button("關閉").clicked() {
                        self.notice = None;
                    }
                });
        }
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.poll_job();
        self.handle_dropped_files(&context);
        self.tick_preview();
        self.top_bar(ui);
        self.bottom_status(ui);
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(13, 18, 26))
                    .inner_margin(22.0),
            )
            .show_inside(ui, |ui| match self.workspace {
                Workspace::Cards => self.cards_workspace(ui, &context),
                Workspace::Overlay => self.overlay_workspace(ui),
            });
        self.popup_messages(&context);
        if self.busy || self.preview_playing {
            context.request_repaint_after(if self.preview_playing {
                Duration::from_secs_f64(1.0 / self.project.fps.clamp(1, 30) as f64)
            } else {
                Duration::from_millis(80)
            });
        }
    }
}

fn install_traditional_chinese_font(context: &egui::Context) -> bool {
    let mut font_dirs = Vec::new();
    if let Some(windows_dir) = std::env::var_os("WINDIR") {
        font_dirs.push(PathBuf::from(windows_dir).join("Fonts"));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        font_dirs.push(
            PathBuf::from(local_app_data)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts"),
        );
    }

    for font_name in ["msjh.ttc", "NotoSansTC-VF.ttf", "mingliu.ttc", "msyh.ttc"] {
        for font_dir in &font_dirs {
            let path = font_dir.join(font_name);
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };

            let mut definitions = egui::FontDefinitions::default();
            let font_name = "system-traditional-chinese".to_owned();
            definitions.font_data.insert(
                font_name.clone(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(family_fonts) = definitions.families.get_mut(&family) {
                    family_fonts.push(font_name.clone());
                }
            }
            context.set_fonts(definitions);
            return true;
        }
    }

    false
}

fn primary_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(
        RichText::new(label)
            .strong()
            .color(Color32::from_rgb(9, 25, 29)),
    )
    .fill(Color32::from_rgb(93, 225, 195))
    .stroke(Stroke::new(1.0, Color32::from_rgb(115, 245, 215)))
    .corner_radius(egui::CornerRadius::same(9))
    .min_size(Vec2::new(135.0, 34.0))
}

fn secondary_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::from_rgb(202, 214, 226)))
        .fill(Color32::from_rgb(27, 38, 52))
        .stroke(Stroke::new(1.0, Color32::from_rgb(47, 64, 83)))
        .corner_radius(egui::CornerRadius::same(8))
}

fn fit_size(source: Vec2, bounds: Vec2) -> Vec2 {
    let scale = (bounds.x / source.x).min(bounds.y / source.y).min(1.0);
    source * scale
}

fn flatten_to_background(foreground: &RgbaImage, background: &RgbaImage) -> RgbaImage {
    RgbaImage::from_fn(foreground.width(), foreground.height(), |x, y| {
        let front = foreground.get_pixel(x, y).0;
        let back = background.get_pixel(x, y).0;
        let alpha = front[3] as f32 / 255.0;
        let mut output = [0_u8; 4];
        for channel in 0..3 {
            output[channel] = (front[channel] as f32 * alpha + back[channel] as f32 * (1.0 - alpha))
                .round() as u8;
        }
        output[3] = 255;
        Rgba(output)
    })
}

fn supported_image(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        ["png", "jpg", "jpeg", "webp", "bmp", "gif"]
            .iter()
            .any(|allowed| extension.eq_ignore_ascii_case(allowed))
    })
}
