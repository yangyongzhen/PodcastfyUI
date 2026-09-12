# 上架前硬伤清单

> 审计时间：2026-09-12。四项**均未动工**，本文件只记录「现状 / 缺口 / 待你拍板」，不替代决策。
> 现状部分标注了可复现命令，便于随时复核，不写口头结论。

## 1. ffmpeg 随包分发（sidecar）

- **现状**：`src-tauri/src/audio/mod.rs` 在运行时定位 ffmpeg（优先可执行文件同目录，其次 PATH）；
  `src-tauri/tauri.conf.json` 的 `bundle` 段**未声明 `externalBin`**。
  复现：`grep -n "externalBin" src-tauri/tauri.conf.json`（无输出即未配置）。
- **缺口**：目标机器没装 ffmpeg 时，四阶段管道的音频合成阶段直接失败；deb/rpm/AppImage 也不会自带。
- **待拍板**：是否随包分发；若分发，用哪种构建——
  **LGPL 构建适合闭源分发，GPL 构建会让整个应用受 GPL 约束**。
- **建议默认**：随包分发 LGPL 构建，声明 `bundle.externalBin`，并在设置页「测试连接」旁加一条 ffmpeg 可用性自检。

## 2. 许可证口径统一

- **现状**：仓库内**没有任何 LICENSE 文件**。复现：`ls LICENSE*` / `glob **/LICENSE*` 均无匹配。
  `package.json`、`src-tauri/Cargo.toml` 与 `README.md` 各自提到许可证，三处口径本次未逐字核对。
- **缺口**：商店上架要求明确许可证；且第 1 项的分发方式会反过来约束许可证选择（见上）。
- **待拍板**：定哪一个——Apache-2.0 / MIT / 双许可。
- **建议默认**：与上游 podcastfy 保持一致（Apache-2.0），补 LICENSE 文件并统一三处声明。

## 3. API key 迁 OS keyring

- **现状**：密钥以**明文 JSON** 存放在应用数据目录，仅依赖文件权限（0600）保护，
  实现在 `src-tauri/src/config/mod.rs`。复现：`grep -n "api_key\|set_permissions" src-tauri/src/config/mod.rs`。
- **缺口**：同机任意进程可读；备份/同步目录会连带外泄；商店审核倾向使用系统凭据库。
- **待拍板**：是否引入 `keyring` crate（Linux secret-service / Windows 凭据管理器 / macOS Keychain），
  以及是否需要「读旧明文 → 迁入 keyring → 删明文」的兼容路径。
- **建议默认**：引入 keyring，首次读取时自动迁移并删除明文文件。

## 4. 隐私声明

- **现状**：`docs/` 下只有 api / architecture / devlog / modules 四份；代码中**未发现**遥测或统计上报
  （`telemetry|analytics|sentry|posthog` 无实质匹配）。
- **缺口**：需向用户说明离开本机的数据只有「你配置的 LLM 与 TTS 请求」（含待转换正文），无账号体系、无遥测。
- **待拍板**：放 README 段落还是独立 `PRIVACY.md`；是否随安装包一并分发。
- **建议默认**：独立 `PRIVACY.md`，README 顶部加链接。

## 建议顺序

1. 定许可证口径 → 2. 定 ffmpeg 分发方式（与许可证互相约束，建议同一轮拍板）→ 3. keyring 迁移 → 4. 隐私声明

## 进展（2026-09-12 更新）

| 项 | 状态 | 落地内容 |
| --- | --- | --- |
| 1 ffmpeg 分发 | ✅ 已定方案并实现 | 你选择**不自带**：改为启动自检——`test_connection("ffmpeg")` 复用 `crate::audio::find_ffmpeg()`（探针的失败点与真实合成路径一致），实际执行一次 `-version` 验证可运行，缺失则给出可操作报错；设置页新增「测试 FFmpeg」按钮 |
| 2 许可证 | ✅ 已完成 | 你选定 **Apache-2.0**：补 `LICENSE`（202 行规范全文）、`package.json` 与 `src-tauri/Cargo.toml` 声明统一、`bundle.licenseFile` 指向 `../LICENSE` |
| 3 API key → keyring | ✅ 已定方案（暂不迁移） | 你选择**保持明文 + 显式提示**：不引入 keyring（规避无 secret-service 的 Linux 机器写不进凭据库的风险），改为设置页密钥区明示「明文存放、仅文件权限保护」，并在 `PRIVACY.md` 如实披露 |
| 4 隐私声明 | ✅ 已完成 | 新增 `PRIVACY.md`：无遥测、无账号；离开本机的只有你配置的 LLM/TTS 请求；并如实披露当前密钥为明文存储 |
