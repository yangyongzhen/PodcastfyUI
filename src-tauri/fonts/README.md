# 随包中文字体

## 为什么需要

视频导出要在画面里渲染中文（播客名、主持人名、字幕）。ffmpeg 的 `drawtext` / `subtitles` 滤镜需要**一个真实的字体文件**才能画出汉字——它不会替你挑系统默认字体。

而不同机器上可用的中文字体差异极大：本机（无头 Linux 容器）`fc-list :lang=zh` 只有 1 个条目，很多精简系统里是 0 个。所以随包携带一份，保证在任何机器上导出视频都不会出现方块/豆腐字。

## 文件

| 项 | 值 |
| --- | --- |
| 文件 | `DroidSansFallbackFull.ttf` |
| 大小 | 4,033,420 字节 |
| SHA-256 | `acb6440a713d880a13a21b468ba7cd43f5a2b2934972e51be791c880730777b8` |
| 来源 | Debian/Ubuntu 包 `fonts-droid-fallback`，安装路径 `/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf` |
| 版权 | Copyright © 2006–2010 Google Corp.（另有 Simon Ochsenreither、Christian Perrier、Vasudev Kamath 等贡献者） |
| 许可证 | **Apache-2.0**（与项目许可证一致，可自由再分发） |

复现来源与校验：

```bash
dpkg -S /usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf   # -> fonts-droid-fallback
grep -i '^License' /usr/share/doc/fonts-droid-fallback/copyright     # -> Apache-2
sha256sum src-tauri/fonts/DroidSansFallbackFull.ttf                  # 比对上表
```

## 打包与运行时定位

`src-tauri/tauri.conf.json` 的 `bundle.resources` 声明了 `fonts/*`，安装后会落到应用资源目录。
运行时查找顺序（见 `src-tauri/src/video/mod.rs`）：

1. 用户在设置页指定的自定义字体路径（`video.font_path`）；
2. 应用资源目录下的 `fonts/DroidSansFallbackFull.ttf`（打包安装后）；
3. 仓库内 `src-tauri/fonts/DroidSansFallbackFull.ttf`（开发期，直接跑 dev / example 时）。

三者都找不到时**提前报错**并给出可操作提示，而不是让 ffmpeg 画出一屏方块。

## 换字体

想换成更漂亮的开源字体（如 Noto Sans SC、思源黑体，均为 OFL 许可）：替换本目录下的 `.ttf`，同步修改 `video::bundled_font()` 中的文件名，并更新上表的 SHA-256 与来源信息。
