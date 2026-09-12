# 用纯 Rust 重写 Podcastfy：把网页、PDF 和一段文字，变成一档双人对话播客

> 一个 Tauri + Svelte 桌面应用的实现记录：不装 Python、不下载模型、不碰 CUDA，
> 只用「编排 + 文本处理 + ffmpeg」把素材变成一段能听的播客。

## 一、先说说这东西是干什么的

如果你用过 Google 的 NotebookLM，大概记得它那个「生成播客」的功能：把几篇文档丢进去，
它会生成一段两个主持人你来我往的**对话式播客**——不是机械地朗读稿子，而是有问有答、
有追问、有总结的那种。

`souzatharsis/podcastfy` 就是这件事的开源复刻版（Python 实现）。它能吃网页、PDF、YouTube、
纯文本，甚至只给一个主题，然后吐出一个 MP3。但它是个**库/命令行工具**：要建 Python 环境、
装一堆依赖、写脚本调用。对普通用户来说，门槛不低。

于是有了 **PodcastfyUI**：

| | |
|---|---|
| **做了什么** | 把 podcastfy 的核心能力重写成**桌面图形应用**（复制粘贴链接就能用） |
| **怎么做的** | 核心引擎用**纯 Rust 重写**，UI 用 **Tauri v2 + Svelte 5** |
| **不做什么** | 不做本地推理——本机不跑任何模型，只负责取内容、编排对话、拼音频 |

![PodcastfyUI 首页](screenshots/01-home.png)

## 二、它凭什么"轻"？——一条关键判断

绝大多数"本地 AI 应用"的复杂度都来自**模型**：权重下载、显存、CUDA、推理后端版本地狱。
但仔细拆一下「生成播客」这件事，会发现真正的计算其实都在远端：

```
内容抽取  = HTTP 抓取 + HTML 正文提取 + PDF 文本提取   ← 纯文本处理，无模型
对话稿生成 = 一次（或几次）LLM API 调用                ← 远端
语音合成  = 每行台词一次 TTS API 调用（N 次）          ← 远端
音频拼接  = ffmpeg concat                              ← 本地，但 ffmpeg 是现成二进制
```

**没有一步需要本地模型。** 于是"纯 Rust 复刻"这条路是成立的：本机只需要做
"编排（orchestration）"。这就是整个项目的技术前提，也是它安装体积小、启动快、
不需要 GPU 的根本原因。

> 一句话定位：**只做编排，不做推理。**

## 三、技术栈与架构

| 层 | 选型 | 说明 |
|---|---|---|
| 桌面壳 | **Tauri v2** | 系统 WebView + Rust 后端，打包体积远小于 Electron |
| 前端 | **Svelte 5（runes）** | `$state` / `$derived` / `$props`，无虚拟 DOM |
| 核心引擎 | **纯 Rust** | 抽取、生成、TTS、拼接、队列全在后端 |
| 进程间 | Tauri 命令 + 事件 | 前端发命令、后端推进度事件 |
| 音频 | **系统 ffmpeg** | 不自带，启动时自检 |

后端模块大致长这样：

```
src-tauri/src/
├── extractor/   内容抽取：网页正文（readability）、PDF（pdf-extract）、YouTube 字幕
├── generator/   对话稿生成：LLM 客户端（OpenAI 兼容 / Anthropic / Gemini / Ollama）
├── tts/         语音合成：openai / edge / doubao 三个 provider，统一 trait
├── audio/       ffmpeg 拼接与探测（find_ffmpeg、concat、时长解析）
├── queue/       四阶段任务流水线，进度事件、取消、失败隔离
├── config/      密钥、LLM、会话配置的读写（api_keys.json / llm.json / conversation.yaml）
└── health.rs    自检探针：一键测 LLM / TTS / ffmpeg 连通性
```

设计上刻意把 **`-- ③ 每一行台词合成一个 mp3 --`** 当作 TTS 层的统一契约：
不管后面接的是 OpenAI、Edge 还是豆包，provider 只需实现"给我一行文本和一个音色，
还我一个 mp3"。这让「三套协议差异巨大的 TTS 服务」在前端看起来是同一件事。

## 四、四阶段流水线

```mermaid
flowchart LR
  A["输入<br/>网页 / YouTube / PDF / 文本 / 主题"] --> B["① 内容抽取<br/>extractor"]
  B --> C["② 对话稿生成<br/>LLM → 主持人 1 / 主持人 2"]
  C --> D["③ 逐行语音合成<br/>TTS（每行一个 mp3）"]
  D --> E["④ ffmpeg 拼接<br/>最终 mp3"]
  E --> F["任务卡片<br/>播放器 / 转录稿编辑"]
  F -.->|只重新合成音频| D
```

几个刻意的设计选择：

- **阶段独立、可回退**：抽取失败不影响已生成的对话稿；转录稿改完可以只重跑 ③④，
  跳过最费钱的 ②（LLM 调用按 token 计费，重跑一次不便宜）。
- **逐行合成而不是整段合成**：虽然调用次数多，但换来三件好事——两位主持人可以用
  不同音色（不然听不出对话感）、单行失败可重试、进度可精确到行。
- **进度即事件**：每个阶段都往前端推状态，任务卡片上有实时进度、可取消、可删除。

## 五、三个值得单独讲的实现细节

### 1. 豆包（火山引擎）TTS：WS 二进制帧 + gzip

OpenAI 的 TTS 就是一个 `POST /v1/audio/speech`，Edge 的接口也无非是 WebSocket 里塞 SSML。
火山引擎（豆包）这套则完全是另一个量级——它的**自定义二进制帧协议**：

```
字节 0: (protocol_version << 4) | header_size
字节 1: (message_type    << 4) | flags
字节 2: (serialization   << 4) | compression
字节 3: reserved
字节 4..8: payload 长度（大端 u32）
字节 8..: payload（gzip 压缩后的 JSON）
```

请求 JSON 形如：

```json
{
  "app":   { "appid": "...", "token": "...", "cluster": "volcano_tts" },
  "user":  { "uid": "podcastfyui" },
  "audio": { "voice_type": "zh_female_wanwanxiaohe_moon_bigtts",
             "encoding": "mp3", "speed_ratio": 1.0 },
  "request": { "reqid": "...", "text": "要说的话", "operation": "query" }
}
```

好消息是：服务端直接回 mp3 分片，**不需要自己解码或重采样**——正好对上本项目
"每行台词一个 mp3" 的统一契约，接入成本比想象中低。

### 2. 一个 403 的定位过程（`[resource_id=] requested resource not granted`）

真实调用时我们收到的是这样一个错误：

```json
{"message":"[resource_id=] requested resource not granted",
 "code":403,"backend_code":45000030}
```

第一步不是改代码，而是**分清"连不通"还是"被拒绝"**——直接对端点做一次裸 WebSocket
握手探测：

```bash
printf 'GET /api/v1/tts/ws_binary HTTP/1.1\r\nHost: openspeech.bytedance.com\r\nUpgrade: websocket\r\n
Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n' \
 | openssl s_client -quiet -connect openspeech.bytedance.com:443 -servername openspeech.bytedance.com
# → HTTP/1.1 101 Switching Protocols      ← 网络与 TLS 都没问题
```

第二步看错误信息里的关键细节：**`[resource_id=]` 是空的**。也就是说网关拿到了请求，
但不知道要为哪个"资源"计费/授权——因为我们的握手只发了鉴权头：

```
Authorization: Bearer;<access_token>
```

而"豆包语音合成大模型"（`*_bigtts` 那批音色）还必须带上资源路由头：

```
X-Api-App-Id:      <App ID>
X-Api-Access-Key:  <Access Token>
X-Api-Resource-Id: volc.service_type.10029   ← 缺的就是这个
X-Api-Connect-Id:  <uuid>
```

修完之后顺手做了两件事：把 `Resource ID` / `API 版本`（v1 / v3）做成**可配置项**，
并在错误信息里回带 resource id，方便下次一眼定位。**这类第三方协议细节，文档与
参考实现的说法经常对不上，唯一可靠的办法是拿真实凭证打一次，看服务端到底抱怨什么。**

### 3. 中文输出的两个"不自然"

选了简体中文，却发现读出来是"英文腔中文"——两个独立的原因：

- **音色没跟着语言走**：Edge TTS 的默认音色是 `en-US-JennyNeural` / `en-US-EricNeural`。
  输出语言为中文时，若音色仍是英文前缀，自动回退到 `zh-CN-XiaoxiaoNeural` /
  `zh-CN-YunxiNeural`；SSML 里的 `xml:lang` 也从音色前缀推导，而不是写死。
- **素材源没跟着语言走**：抓 YouTube 时优先挑 `zh` 字幕轨道，而不是默认那条。

另外，转录稿的解析规则是 `PERSON_1:` / `PERSON_2:` 前缀（兼容 `P` / `A` / `Q`
这类小写变体），没有前缀的行**静默跳过**——LLM 有时会写旁白，宁可丢掉也不要让它
把旁白念成"主持人"。

## 六、界面

任务列表：四阶段实时进度、可取消/删除，完成后直接播放。

![任务列表与四阶段实时进度](screenshots/02-tasks.png)

设置页：密钥按所选 provider 就近显示（选 OpenAI 才出现 OpenAI key），
豆包凭证与 Resource ID 只在选用豆包 TTS 时出现——**不该填的东西默认不出现**，
避免"这是不是必填"的误导。

![设置页：密钥按所选 provider 就近显示](screenshots/03-settings.png)

其余体验细节：LLM / TTS / ffmpeg 的自检探针、中英双语界面、转写稿双色渲染
（两位主持人不同色）、完成后的"仅重新合成音频"按钮。

## 七、能力边界（先说清楚，免得踩坑）

一个诚实的功能列表，应该同时说清"它不做什么"：

| 边界 | 说明 |
|---|---|
| **不做本地推理** | 所有生成都调远端 LLM / TTS，本机不跑模型。离线只能改稿，不能生成 |
| **ffmpeg 不自带** | 依赖系统 `ffmpeg`，启动时自检并明确提示，不静默失败 |
| **Edge TTS 是免费公共接口** | 无需 key，但受微软服务波动影响。我们实测过：`/voices/list` 返回 200，**合成却返回 400**——"接口通"不等于"能合成" |
| **豆包需控制台开通** | 只在控制台拿到 App ID / Token 还不够，必须在火山控制台**开通「大模型语音合成」并激活对应音色**，否则报无权限（45000030 一类的错误） |
| **API key 明文存储** | 存在本机应用数据目录（靠文件权限保护），设置页有显式提示，见 `PRIVACY.md`。不引入系统密钥环是刻意的取舍 |
| **任务不跨重启持久化** | 任务列表在内存中，重启后需重新发起（已在路线图） |

关于**怎么验证**，也有个原则：真要证明"链路是通的"，得用真实 LLM 产出真实对话稿，
而 TTS 在无凭证时用替身（stub）隔离——这样能确认抽取 → 生成 → 拼接三段无误，
**但绝不把替身的结果说成"语音合成成功"**。冒烟测试的产物是要能验的：
`mp3 / 24000 Hz / mono`，改稿后"仅重新合成音频"能独立跑通。

## 八、快速开始

前置：**Rust** 工具链、**Node 18+**、系统已装 **ffmpeg**。

```bash
git clone https://gitcode.com/qq8864/PodcastfyUI.git
cd PodcastfyUI

npm install          # 首次安装前端依赖

npm run tauri dev    # 开发模式（热重载）
npm run tauri build  # 生产构建，产物在 src-tauri/target/release/bundle/
```

首次启动进 **设置** 页完成三项配置，点「测试连接」确认全绿：

1. **LLM**：`openai`（兼容任意 OpenAI 接口，填 Base URL + 模型名即可接自有网关或中转）、
   `anthropic`、`gemini`，或本地 `ollama`（默认 `http://localhost:11434/v1`，无需 key）。
2. **TTS**：`edge` 只需选音色（免费免 key）；`openai` 需 key；`doubao` 需 App ID +
   Access Token + Resource ID + 两个音色。
3. **输出语言**：`简体中文` / `繁體中文` / `English`，选中文会自动换中文音色与中文字幕轨道。

## 九、路线

- **已完成（MVP，2026-09-12）**：`cargo check` ✅ · `cargo test` ✅ ·
  `svelte-check` 0 错 0 警 ✅ · `vite build` ✅。
- **进行中**：豆包 TTS 真实凭证联调（已修掉缺 `X-Api-Resource-Id` 导致的 403）。
- **下一步**：任务持久化 · ElevenLabs / Gemini 多说话人 TTS · 拖拽文件输入 ·
  应用图标与各平台打包。
- **上架计划**：Linux 先行（deb / rpm / AppImage）→ Windows（MSIX 或静默安装 +
  代码签名 + 离线 WebView2）→ **鸿蒙单独立项**（`.hap` + ArkTS 重写 UI，Rust 走 NAPI）。
- **License**：Apache-2.0（与上游 podcastfy 一致）。

## 十、写在最后

这个项目有意思的地方不在"用了什么新框架"，而在**一次由问题驱动的减法**：
把"本地 AI 应用"必须背的模型包袱全部拿掉，只留下真正需要本机做的事——取内容、
编排对话、拼接音频；把重活交给远端 API，把体验交给 Tauri + Svelte。

剩下的复杂度，就只剩下**那些琐碎但真实的第三方协议细节**：一个缺失的
`X-Api-Resource-Id` 头、一个"接口通但合成不通"的免费服务、一个语言和音色不同步
引发的"英文腔中文"。这些东西文档里通常不会写，只能靠真实调用一点点撞出来——
而这也正是把项目跑通的乐趣所在。

**仓库**：<https://gitcode.com/qq8864/PodcastfyUI>

更多文档：`docs/architecture.md`（架构设计）· `docs/modules.md`（模块职责）·
`docs/api.md`（命令/事件 API）· `docs/devlog.md`（开发日志与踩坑）·
`docs/release-readiness.md`（上架就绪度）。
