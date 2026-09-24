# 專案重寫工作計畫

## 目標
將現有透明特效字卡／疊圖影片流程以 Rust 重寫，加入 GUI，讓使用者能快速替換圖片、調整效果並產出動態影片。

## 階段
1. 盤點現有專案、輸入輸出、視覺效果與執行方式 — 完成
2. 定義 Rust 架構與 GUI 最小可用流程 — 完成
3. 實作影片合成、素材替換、預覽與輸出 GUI — 完成
4. 建置已通過；原生 GUI 點操作未完成（工具無 Windows app API）
5. 整理使用說明與目前能力基線 — 完成

## 決策與限制
- 保留現有視覺效果與主要輸出行為，依盤點結果調整實作。
- 優先用可本機執行、可逆的 Rust/GUI 方案。
- 未經實際啟動與操作的功能不標記為 PASS。

## 錯誤紀錄
| 錯誤 | 嘗試 | 處理 |
|---|---:|---|
| eframe App API／型別推導編譯錯誤 | 1 | 依 eframe 0.34 實際 App/Panel API 修正；改正 egui 路徑、浮點負號與 ffprobe 字串生命週期 |
| Computer Use docs 路徑錯誤 | 1 | 使用技能檔宣告的 `../../docs` 相對於技能目錄重算實際路徑 |
- PowerShell 檢視原始碼區段曾使用不支援的 `Select-Object -Index 95..104` 寫法；改用 `for` 迴圈區段讀取。eFrame 套件原始碼位置是 `src/epi.rs`，不含額外 `epi\mod.rs`。
| Rust formatting check | 1 | `cargo fmt --check` 顯示 rustfmt 差異；執行 rustfmt 後再檢查 |
| Native GUI acceptance API unavailable | 1 | `cua.getState()` 顯示 `apps: []`；`listApps`/`listWindows` 未提供，無法操作原生窗口，改採可用的 release build 和啟動證據並明確揭露範圍 |
| Image API source lookup | 1 | Tried guessed `src/io/reader.rs` and `src/dynimage.rs` paths; use `rg --files` in the installed crate to find the current module layout |
| EXIF API 匯入名稱錯誤 | 1 | image 0.25.10 的 Orientation 位於 `image::metadata::Orientation`；改用該路徑並重新完成 fmt/check/release build |
