# 透明字卡工作室

以 Rust 製作的 Windows 桌面 GUI，用來快速替換透明圖片、套用循環動畫並輸出透明影片。FFmpeg 負責影片編碼和主影片合成。

App 圖示位於 `assets/app_icon.png`，Windows 執行檔使用多尺寸 `assets/app_icon.ico`；建置時會自動嵌入圖示。

## 開始使用

需要 Rust stable 與 FFmpeg。Windows MSVC 建置另需 Windows SDK 的 `rc.exe` 以嵌入圖示。FFmpeg 必須在 PATH 中；若使用自訂位置，可設定環境變數 FFMPEG 指向執行檔。

GUI 會自動載入 Windows 已安裝的繁體中文字型（例如 Microsoft JhengHei 或 Noto Sans TC）。若文字仍顯示方框，請先在 Windows 安裝其中一種字型再重新開啟程式。

在專案資料夾開啟 PowerShell：

    cargo run --release

或先建立可執行檔：

    cargo build --release
    .\target\release\transparent-overlay-studio.exe

GUI 內附 3 張範例圖片，可先按「載入範例」檢查轉場與透明度。也可以把 PNG、JPEG、WebP、BMP 或 GIF 拖進素材清單；JPEG 會依 EXIF 方向資訊自動旋轉，GIF 會讀取第一格。

素材清單上方會依目前畫布顯示建議原圖尺寸。建議使用有透明背景的 PNG 或 WebP；其他支援格式也能匯入，但白底不會自動去背。圖片會保持比例縮放並置中，最大約佔畫布寬高的 64%。

## 字卡製作

素材清單支援加入多張圖片、替換選取圖片、移除及上下調整順序。儲存專案後，下次載入專案即可沿用尺寸、影格率、效果和輸出設定，直接替換素材。

動畫效果：

- **透明輪播**：依清單順序切換圖片，最後一張會轉場回第一張。每張圖片的設定秒數已包含轉場時間。
- **漂浮循環**：第一張圖片沿著柔和路徑移動。
- **旋轉循環**：第一張圖片旋轉一整圈，會保留透明邊界。

輸出可選 960×540 等偶數尺寸、1–120 fps、每張圖片秒數與轉場時間。圖片保持比例置中，限制在畫布約 64% 的範圍內。轉場在預乘 Alpha 空間混合，成品使用直通 Alpha。

支援格式：

| 格式 | 說明 |
|---|---|
| ProRes 4444 MOV | 通用透明影片母檔 |
| QTRLE MOV | 無損保存 8-bit RGBA |
| VP9 WebM | 體積較小的有損透明版本 |

可選擇同時輸出棋盤格 MP4 預覽及 PNG 逐格素材。棋盤格 MP4 是不透明的檢查影片，不是透明素材。輸出資料夾必須是新資料夾或空資料夾；程式不會覆蓋其中既有檔案。透明 sample 和 manifest 會一併輸出。

右側預覽可按「播放／暫停」，也可拖曳影格；輸出透明動畫完成後會自動從頭播放同一組圖片與效果的循環預覽。預覽使用 App 內的影格渲染，棋盤格只是檢查透明區域的背景。

來源圖片必須已經具有透明背景。白底 JPEG 或完全不透明 PNG 不會自動去背，程式會顯示提醒。

## 疊入主影片

切到「疊入主影片」工作區，選擇主影片、已輸出的透明素材和 MP4 輸出位置。可以調整左上角座標、縮放寬度、開始和結束時間；預設沿用主影片影格率並保留主片聲音。

## FFmpeg 設定

FFmpeg 必須包含所選格式的 encoder：

- ProRes：prores_ks
- QTRLE：qtrle
- WebM：libvpx-vp9
- 棋盤格預覽與主影片輸出：libx264

PowerShell 使用自訂 FFmpeg 位置的範例：

    $env:FFMPEG = 'C:\ffmpeg\bin\ffmpeg.exe'
    cargo run --release

## 建置

目前以 Rust 1.94.1、FFmpeg 8.1.1 在 Windows 環境建置。已有相依套件快取時，可離線檢查及建置：

    cargo check --offline
    cargo build --release --offline

專案根目錄仍保留舊版 Python 命令列腳本與原輸出樣本，供既有工作流程參考；新的主要入口是 Rust GUI。
