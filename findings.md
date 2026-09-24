# 研究發現

## 已確認
- 專案目前不是 Git repository，根目錄沒有 Rust/Cargo 專案。
- 現有核心是 `make_overlay.py`：Python + Pillow 逐幀合成，呼叫 FFmpeg 編碼；支援透明 PNG/JPEG 載入、EXIF 方向修正、固定 64% fit；輪播會最後一張轉回第一張，轉場在預乘 Alpha 空間做 smoothstep 漸變並水平移動；單圖另有 float/spin。輸出 straight-alpha ProRes 4444、QTRLE、VP9 WebM，可保留 PNG 幀並產生棋盤格 MP4 預覽及 manifest。
- `overlay_on_video.py` 將透明素材循環疊到一般影片，支援 x/y/width/start/end/fps，保存主片音訊輸出 H.264/AAC MP4。
- 生成條件：畫布偶數且至少 64x64，fps 1..120，時長乘 fps 必須是整數幀；輪播轉場至少 1 幀且小於單張時段。預設 960x540、30 fps、每圖 3 秒（包含 1 秒轉場）。非空輸出資料夾受保護。
- 樣本資料在 `samples/`，包含 180 張 frame PNG、3 張透明素材圖、ProRes/QTRLE/WebM 與不透明棋盤格預覽；這些是既有可檢視的產品輸出樣本，不是輸入素材。
- 本機有 Rust 1.94.1、cargo 1.94.1、FFmpeg/ffprobe 8.1.1；Cargo source cache 有 eframe 0.34.2、image 0.25.10、rfd 0.17.2、serde、anyhow 等 crate，可嘗試 offline build。

## 設計決定
- GUI 採 Rust 原生桌面 egui/eframe；動畫 RGBA 合成與參數驗證放在 Rust module，FFmpeg 作為外部 encoder/主片合成器。
- MVP GUI 包含素材加入／替換／移除／排序、三種動畫模式、畫布／fps／時長／轉場設定、三種透明格式、輸出資料夾選擇與背景輸出進度。
- 將加入專案 JSON 儲存/載入，以便下次快速換圖沿用輸出參數；主片疊加流程也會在 GUI 提供入口，對應原有功能。
- GUI 預覽先提供棋盤背景與時間位置滑桿，並依 Rust renderer 產生當前影格；不做逐幀影片播放器控制。

## 推論／未知
- 目前 package source cache 看起來包含直接依賴，但尚未確認完整 build dependency cache；後續以 `cargo check --offline`/build 結果定界。
- eframe 0.34 native Windows 視窗與對話框需實際操作驗收。
- 若原始透明圖格式包含 TIFF/PSD/動態 GIF，本版以 `image` crate 可讀的常見平面圖為範圍，未承諾保留多頁/動畫資訊。

## 執行環境查詢提醒
- 查版本時本機 FFmpeg 指令採 `-version`；曾誤以 `--version` 呼叫一次，產生 option error，已改用正確旗標得到完整版本資訊。
- eframe 0.34.2 supports a smaller OpenGL-native feature set (`glow`, `default_fonts`, platform window dependencies); rfd 0.17.2 exposes Windows common controls and optional Linux portal features.
- GUI architecture now has two tasks: (1) produce alpha animation masters, (2) overlay the same animation onto a base video with timeline placement and audio preservation. Core renders RGBA in Rust; FFmpeg handles only encoding/probing/final compositing.
