//! 豆包（火山引擎）TTS provider。
//!
//! 同时支持两套 WebSocket 协议，用配置项 `doubao_api_version` 选择：
//!
//! * `v1`（默认）—— 老协议：`/api/v1/tts/ws_binary`，payload 为
//!   `{"app":{appid,token,cluster},"user":{},"audio":{...},"request":{...}}`。
//! * `v3` —— 新协议（事件式）：`/api/v3/tts/bidirection`，payload 为
//!   `{"user":{},"namespace":"BidirectionalTTS","req_params":{speaker,audio_params,text}}`。
//!
//! **关键修复**：无论哪套协议，握手都必须带上网关路由用的 Header，尤其是
//! `X-Api-Resource-Id`。只发鉴权头（值为 `Bearer;` 拼 token）时网关拿不到
//! resource_id，会回 `[resource_id=] requested resource not granted`（403 /
//! backend_code 45000030）。大模型 TTS（bigtts 音色）对应
//! `volc.service_type.10029`。
//!
//! V1/V3 帧格式一致：`[4B 头][可选字段][4B payload 长度][payload]`，见
//! <https://www.volcengine.com/docs/6561/1257584>（V1）与
//! <https://www.volcengine.com/docs/6561/1719100>（V3）。
//!
//! 与本项目其它 provider 的差异：`synthesize_line` 每行输出一个 mp3 文件。
//! 请求里的音频格式为 `mp3`，服务端返回的就是 mp3 分片，直接把收到的音频字节
//! 按顺序拼接后写入 `out_path` 即可，**不需要解码 / 重采样**。

use anyhow::{anyhow, bail};
use async_trait::async_trait;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, Request};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tracing::{error, info};

/// 豆包 TTS V1（ws_binary）接入点。
const DOUBAO_TTS_V1_URL: &str = "wss://openspeech.bytedance.com/api/v1/tts/ws_binary";
/// 豆包 TTS V3（双向流式 / 事件式）接入点。
const DOUBAO_TTS_V3_URL: &str = "wss://openspeech.bytedance.com/api/v3/tts/bidirection";

/// 大模型 TTS（bigtts 音色）默认 resource id。
pub const DEFAULT_RESOURCE_ID: &str = "volc.service_type.10029";
/// 默认协议版本。
pub const DEFAULT_API_VERSION: &str = "v1";

// 帧类型（V1/V3 共用同一套二进制协议）
const CLIENT_FULL_REQUEST: u8 = 0x1; // 客户端全量请求
const SERVER_FULL_RESPONSE: u8 = 0x9; // 服务端全量响应（V3 事件帧）
const SERVER_AUDIO_RESPONSE: u8 = 0xB; // 服务端音频响应
const SERVER_ERROR_RESPONSE: u8 = 0xF; // 服务端错误响应
const SERIALIZATION_JSON: u8 = 0x1; // payload 为 JSON
const COMPRESSION_NONE: u8 = 0x0; // payload 不压缩
const COMPRESSION_GZIP: u8 = 0x1; // payload 用 gzip 压缩
const FLAG_WITH_EVENT: u8 = 0x4; // V3：payload 前带 event 号

// V3 事件号
const EVENT_START_CONNECTION: i32 = 1;
const EVENT_FINISH_CONNECTION: i32 = 2;
const EVENT_CONNECTION_STARTED: i32 = 50;
const EVENT_CONNECTION_FAILED: i32 = 51;
const EVENT_START_SESSION: i32 = 100;
const EVENT_FINISH_SESSION: i32 = 102;
const EVENT_SESSION_STARTED: i32 = 150;
const EVENT_SESSION_FINISHED: i32 = 152;
const EVENT_SESSION_FAILED: i32 = 153;
const EVENT_TASK_REQUEST: i32 = 200;
const EVENT_TTS_ENDED: i32 = 359;

/// 连接级事件在线上**不携带** session id。
fn is_connection_event(event: i32) -> bool {
    matches!(
        event,
        EVENT_START_CONNECTION
            | EVENT_FINISH_CONNECTION
            | EVENT_CONNECTION_STARTED
            | EVENT_CONNECTION_FAILED
            | 52
    )
}

/// 豆包（火山引擎）语音合成 provider。
pub struct DoubaoTts {
    app_id: String,
    access_token: String,
    /// 集群名（火山控制台的 cluster），仅 V1 使用，本项目默认 `volcano_tts`。
    cluster: String,
    /// 默认音色（voice_type）；调用时可逐行覆盖。
    default_voice: String,
    /// 网关路由用的资源 id（`X-Api-Resource-Id`）。
    resource_id: String,
    /// 协议版本："v1" | "v3"。
    api_version: String,
}

/// tungstenite 的 WS 流类型（connect_async 的返回类型）。
type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

impl DoubaoTts {
    pub fn new(
        app_id: String,
        access_token: String,
        cluster: String,
        default_voice: String,
        resource_id: String,
        api_version: String,
    ) -> Self {
        Self {
            app_id,
            access_token,
            cluster,
            default_voice,
            resource_id,
            api_version,
        }
    }

    fn is_v3(&self) -> bool {
        self.api_version.eq_ignore_ascii_case("v3")
    }

    fn endpoint(&self) -> &'static str {
        if self.is_v3() {
            DOUBAO_TTS_V3_URL
        } else {
            DOUBAO_TTS_V1_URL
        }
    }

    /// 构造握手请求（含网关路由所需的全部 Header）。抽成纯函数以便单测。
    ///
    /// `into_client_request()` 会自动补齐 Host / Connection / Upgrade /
    /// Sec-WebSocket-Version / Sec-WebSocket-Key，缺少这些头会被 tungstenite
    /// 拒绝；手工 builder 无法补齐。
    fn build_ws_request(&self) -> anyhow::Result<Request<()>> {
        let mut request = self
            .endpoint()
            .into_client_request()
            .map_err(|e| anyhow!("doubao tts: 构造请求失败: {}", e))?;
        let headers = request.headers_mut();

        // 鉴权头：值为 "Bearer;" 拼接 Access Token（分号是协议固定格式，不是笔误）。
        let mut insert = |name: &'static str, value: &str| -> anyhow::Result<()> {
            headers.insert(
                name,
                HeaderValue::from_str(value)
                    .map_err(|e| anyhow!("doubao tts: 请求头 {name} 非法: {e}"))?,
            );
            Ok(())
        };
        insert("Authorization", &format!("Bearer;{}", self.access_token))?;
        // X-Api-* 头是网关按 resource_id 路由 / 鉴权的关键（缺 resource_id 会 403）。
        insert("X-Api-App-Id", &self.app_id)?;
        // 部分端点用 App-Key，两个都发以兼容。
        insert("X-Api-App-Key", &self.app_id)?;
        insert("X-Api-Access-Key", &self.access_token)?;
        insert("X-Api-Resource-Id", &self.resource_id)?;
        insert("X-Api-Connect-Id", &uuid::Uuid::new_v4().to_string())?;
        Ok(request)
    }

    /// 服务端拒绝时的统一错误文案（带上 resource_id 便于定位）。**不打印 token**。
    fn reject_message(&self, detail: impl std::fmt::Display) -> anyhow::Error {
        anyhow!(
            "doubao tts: 服务端拒绝（resource_id={}）: {}",
            self.resource_id,
            detail
        )
    }

    /// 合成整段文本，返回按顺序拼接好的 mp3 字节。
    async fn doubao_tts_to_mp3(&self, text: &str, voice_type: &str) -> anyhow::Result<Vec<u8>> {
        if self.app_id.is_empty() || self.access_token.is_empty() {
            bail!("doubao tts: 需要配置 app_id / access_token");
        }
        if self.is_v3() {
            if self.resource_id.is_empty() {
                bail!("doubao tts: v3 需要配置 resource_id");
            }
            self.synthesize_v3(text, voice_type).await
        } else {
            if self.cluster.is_empty() {
                bail!("doubao tts: 需要配置 app_id / access_token / cluster");
            }
            self.synthesize_v1(text, voice_type).await
        }
    }

    /// V1：单次 full client request，随后流式收音频分片。
    async fn synthesize_v1(&self, text: &str, voice_type: &str) -> anyhow::Result<Vec<u8>> {
        let request = self.build_ws_request()?;
        let (mut ws, _resp) = connect_async(request)
            .await
            .map_err(|e| anyhow!("doubao tts: 连接失败: {}", e))?;

        // 构建 full client request：JSON -> gzip -> [4B 头][4B payload 长度][payload]
        let req = json!({
            "app": {
                "appid": self.app_id,
                "token": self.access_token,
                "cluster": self.cluster,
            },
            "user": { "uid": "podcastfyui" },
            "audio": {
                "voice_type": voice_type,
                "encoding": "mp3",
                "speed_ratio": 1.0,
                "volume_ratio": 1.0,
                "pitch_ratio": 1.0,
            },
            "request": {
                "reqid": uuid::Uuid::new_v4().to_string(),
                "text": text,
                "text_type": "plain",
                "operation": "submit",
            },
        });
        let frame = build_frame(
            CLIENT_FULL_REQUEST,
            0,
            SERIALIZATION_JSON,
            &serde_json::to_vec(&req)?,
        );
        ws.send(WsMessage::Binary(frame)).await?;

        // 循环接收音频帧，直到最后一包（seq < 0）。mp3 分片直接顺序拼接。
        let mut mp3 = Vec::new();
        loop {
            let msg = timeout(Duration::from_secs(60), ws.next())
                .await
                .map_err(|_| anyhow!("doubao tts: 等待音频响应超时"))?
                .ok_or_else(|| anyhow!("doubao tts: 连接在响应完成前关闭"))??;
            if let WsMessage::Binary(data) = msg {
                let (audio, is_last) = parse_audio_response(&data, &self.resource_id)?;
                mp3.extend_from_slice(&audio);
                if is_last {
                    break;
                }
            }
        }
        Ok(mp3)
    }

    /// V3：事件式双向流协议。
    ///
    /// 时序：StartConnection -> ConnectionStarted -> StartSession ->
    /// SessionStarted -> TaskRequest -> FinishSession -> 收音频直到
    /// TTSEnded / SessionFinished -> FinishConnection。
    async fn synthesize_v3(&self, text: &str, voice_type: &str) -> anyhow::Result<Vec<u8>> {
        let request = self.build_ws_request()?;
        let (mut ws, _resp) = connect_async(request)
            .await
            .map_err(|e| anyhow!("doubao tts: 连接失败: {}", e))?;

        let session_id = uuid::Uuid::new_v4().to_string();
        let uid = uuid::Uuid::new_v4().to_string();
        let base = json!({
            "user": { "uid": uid },
            "namespace": "BidirectionalTTS",
            "req_params": {
                "speaker": voice_type,
                "audio_params": {
                    "format": "mp3",
                    "sample_rate": 24000,
                    "speech_rate": 0,
                },
            },
        });

        // 1. StartConnection
        ws.send(WsMessage::Binary(build_event_frame(
            EVENT_START_CONNECTION,
            "",
            b"{}",
        )))
        .await?;
        let f = recv_server_frame(&mut ws).await?;
        if f.event != Some(EVENT_CONNECTION_STARTED) {
            return Err(self.reject_message(format!(
                "未收到 ConnectionStarted（event={:?}）",
                f.event
            )));
        }

        // 2. StartSession
        let mut start = base.clone();
        start["event"] = json!(EVENT_START_SESSION);
        ws.send(WsMessage::Binary(build_event_frame(
            EVENT_START_SESSION,
            &session_id,
            &serde_json::to_vec(&start)?,
        )))
        .await?;
        let f = recv_server_frame(&mut ws).await?;
        if f.event != Some(EVENT_SESSION_STARTED) {
            return Err(self.reject_message(format!(
                "未收到 SessionStarted（event={:?}）",
                f.event
            )));
        }

        // 3. TaskRequest（携带整段文本）+ 立即 FinishSession 表示没有更多文本
        let mut task = base.clone();
        task["event"] = json!(EVENT_TASK_REQUEST);
        task["req_params"]["text"] = json!(text);
        ws.send(WsMessage::Binary(build_event_frame(
            EVENT_TASK_REQUEST,
            &session_id,
            &serde_json::to_vec(&task)?,
        )))
        .await?;
        ws.send(WsMessage::Binary(build_event_frame(
            EVENT_FINISH_SESSION,
            &session_id,
            b"{}",
        )))
        .await?;

        // 4. 接收音频，直到 TTSEnded / SessionFinished
        let mut mp3 = Vec::new();
        loop {
            let f = recv_server_frame(&mut ws).await?;
            match f.msg_type {
                SERVER_AUDIO_RESPONSE => {
                    if !f.payload.is_empty() {
                        mp3.extend_from_slice(&f.payload);
                    }
                }
                SERVER_FULL_RESPONSE => match f.event {
                    Some(EVENT_TTS_ENDED) | Some(EVENT_SESSION_FINISHED) => break,
                    Some(EVENT_SESSION_FAILED) | Some(EVENT_CONNECTION_FAILED) => {
                        return Err(self.reject_message(format!(
                            "服务端事件失败（event={:?}）: {}",
                            f.event,
                            String::from_utf8_lossy(&f.payload)
                        )));
                    }
                    // TTSResponse / 句首句尾等元数据帧，忽略
                    _ => {}
                },
                SERVER_ERROR_RESPONSE => {
                    return Err(self.reject_message(format_error_payload(&f.payload)));
                }
                _ => {}
            }
        }

        // 5. FinishConnection（尽力而为，失败不影响已合成的音频）
        let _ = ws
            .send(WsMessage::Binary(build_event_frame(
                EVENT_FINISH_CONNECTION,
                "",
                b"{}",
            )))
            .await;
        Ok(mp3)
    }
}

#[async_trait]
impl super::TtsProvider for DoubaoTts {
    fn name(&self) -> &'static str {
        "doubao"
    }

    async fn synthesize_line(
        &self,
        text: &str,
        voice: &str,
        out_path: &Path,
    ) -> Result<(), String> {
        let voice_type = if voice.is_empty() {
            self.default_voice.as_str()
        } else {
            voice
        };
        info!(
            "doubao-tts synthesize_line ({} chars, api={})",
            text.chars().count(),
            if self.is_v3() { "v3" } else { "v1" }
        );
        let mp3 = self
            .doubao_tts_to_mp3(text, voice_type)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(parent) = out_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::write(out_path, &mp3)
            .await
            .map_err(|e| format!("write {}: {e}", out_path.display()))
    }
}

/// 构建 V1 二进制帧：头 4 字节 + payload 长度（大端）+ gzip 压缩的 payload。
fn build_frame(message_type: u8, flags: u8, serialization: u8, payload: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(payload).expect("gzip write");
    let compressed = enc.finish().expect("gzip finish");
    let header = [
        (1 << 4) | 1, // protocol_version=1, header_size=1（4 字节头）
        (message_type << 4) | flags,
        (serialization << 4) | COMPRESSION_GZIP,
        0, // reserved
    ];
    let mut frame = Vec::with_capacity(8 + compressed.len());
    frame.extend_from_slice(&header);
    frame.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    frame.extend_from_slice(&compressed);
    frame
}

/// 构建 V3 事件帧（payload 不压缩）：
/// `[4B 头][4B event][(接续事件) 4B sid 长度 + sid][4B payload 长度][payload]`。
fn build_event_frame(event: i32, session_id: &str, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(16 + payload.len());
    frame.extend_from_slice(&[
        (1 << 4) | 1, // version=1, header_size=1
        (CLIENT_FULL_REQUEST << 4) | FLAG_WITH_EVENT,
        (SERIALIZATION_JSON << 4) | COMPRESSION_NONE,
        0, // reserved
    ]);
    frame.extend_from_slice(&event.to_be_bytes());
    if !is_connection_event(event) {
        let sid = session_id.as_bytes();
        frame.extend_from_slice(&(sid.len() as u32).to_be_bytes());
        frame.extend_from_slice(sid);
    }
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// 解析后的服务端帧（V1 与 V3 复用）。
struct ServerFrame {
    msg_type: u8,
    event: Option<i32>,
    payload: Vec<u8>,
}

/// 读一个 u32 大端（带边界检查）。
fn be_u32(data: &[u8], pos: usize) -> anyhow::Result<usize> {
    if data.len() < pos + 4 {
        bail!("doubao tts: 响应帧字段越界");
    }
    Ok(u32::from_be_bytes([
        data[pos],
        data[pos + 1],
        data[pos + 2],
        data[pos + 3],
    ]) as usize)
}

/// gzip 解压（失败则原样返回）。
fn gunzip(data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    if GzDecoder::new(data).read_to_end(&mut buf).is_ok() && !buf.is_empty() {
        buf
    } else {
        data.to_vec()
    }
}

/// 解析 V3 服务端帧（含事件头扩展）。
fn parse_server_frame(data: &[u8]) -> anyhow::Result<ServerFrame> {
    if data.len() < 4 {
        bail!("doubao tts: 响应帧过短");
    }
    let header_size = ((data[0] & 0x0f) as usize) * 4;
    if data.len() < header_size || header_size < 4 {
        bail!("doubao tts: 响应帧头长度非法");
    }
    let msg_type = data[1] >> 4;
    let flags = data[1] & 0x0f;
    let compression = data[2] & 0x0f;

    let mut pos = header_size;
    let mut event = None;
    if flags == FLAG_WITH_EVENT {
        if data.len() < pos + 4 {
            bail!("doubao tts: 事件帧缺少 event");
        }
        let ev = i32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        event = Some(ev);
        pos += 4;
        if is_connection_event(ev) {
            // 服务端连接级事件会带 connect_id（长度 + 字节）。
            if msg_type == SERVER_FULL_RESPONSE {
                let cid_len = be_u32(data, pos)?;
                pos += 4 + cid_len;
            }
        } else {
            let sid_len = be_u32(data, pos)?;
            pos += 4 + sid_len;
        }
    }
    if msg_type == SERVER_ERROR_RESPONSE {
        pos += 4; // error code
    } else if flags == 0x1 || flags == 0x3 {
        pos += 4; // 序列号
    }
    let plen = be_u32(data, pos)?;
    pos += 4;
    let end = pos.saturating_add(plen).min(data.len());
    let mut payload = data[pos..end].to_vec();
    if compression == COMPRESSION_GZIP {
        payload = gunzip(&payload);
    }
    Ok(ServerFrame {
        msg_type,
        event,
        payload,
    })
}

/// 从错误帧 payload 里提取 `[code] msg`。
fn format_error_payload(payload: &[u8]) -> String {
    if payload.len() < 8 {
        return String::from_utf8_lossy(payload).to_string();
    }
    let code = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let msg = String::from_utf8_lossy(&payload[8..]);
    format!("[{code}] {msg}")
}

/// 接收一条服务端帧（自动回应 ping）。
async fn recv_server_frame(ws: &mut WsStream) -> anyhow::Result<ServerFrame> {
    loop {
        let msg = timeout(Duration::from_secs(60), ws.next())
            .await
            .map_err(|_| anyhow!("doubao tts: 等待响应超时"))?
            .ok_or_else(|| anyhow!("doubao tts: 连接在响应完成前关闭"))??;
        match msg {
            WsMessage::Binary(data) => return parse_server_frame(&data),
            WsMessage::Ping(data) => {
                let _ = ws.send(WsMessage::Pong(data)).await;
            }
            WsMessage::Close(_) => bail!("doubao tts: 服务端关闭了连接"),
            _ => {}
        }
    }
}

/// 解析 V1 音频响应帧，返回 (音频字节, 是否最后一包)。
///
/// 音频帧 payload 结构：`[i32 BE seq][u32 size][audio...]`，`seq < 0` 表示末包。
/// 错误帧 payload 结构：`[i32 BE code][u32 size][msg...]`，msg 可能被 gzip 压缩。
fn parse_audio_response(data: &[u8], resource_id: &str) -> anyhow::Result<(Vec<u8>, bool)> {
    if data.len() < 4 {
        bail!("doubao tts: 响应帧过短");
    }
    let message_type = data[1] >> 4;
    let flags = data[1] & 0x0f;
    let head_size = (data[0] & 0x0f) as usize;
    let payload = &data[head_size * 4..];

    match message_type {
        SERVER_AUDIO_RESPONSE => {
            if flags == 0 {
                // 无序列号：本包无音频数据
                return Ok((Vec::new(), false));
            }
            if payload.len() < 8 {
                bail!("doubao tts: 音频数据长度不足");
            }
            let sequence = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let audio = payload[8..].to_vec();
            Ok((audio, sequence < 0))
        }
        SERVER_ERROR_RESPONSE => {
            if payload.len() < 8 {
                bail!("doubao tts: 错误消息数据长度不足");
            }
            let code = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            // 错误消息可能 gzip 压缩
            let err_bytes = gunzip(&payload[8..]);
            let msg = String::from_utf8_lossy(&err_bytes);
            error!(
                "doubao tts server error code={} resource_id={}: {}",
                code, resource_id, msg
            );
            bail!(
                "doubao tts: 服务端拒绝（resource_id={}）: [{}] {}",
                resource_id,
                code,
                msg
            );
        }
        other => bail!("doubao tts: 未知响应类型: {}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_provider(api_version: &str) -> DoubaoTts {
        DoubaoTts::new(
            "1234567890".into(),
            "test-access-key-value".into(),
            "volcano_tts".into(),
            "zh_female_wanwanxiaohe_moon_bigtts".into(),
            DEFAULT_RESOURCE_ID.into(),
            api_version.into(),
        )
    }

    #[test]
    fn test_build_frame_header() {
        let frame = build_frame(CLIENT_FULL_REQUEST, 0, SERIALIZATION_JSON, b"{}");
        // 头 4 字节：版本/头长、消息类型/flags、序列化/压缩、reserved
        assert_eq!(&frame[0..4], &[0x11, 0x10, 0x11, 0x00]);
        // payload 长度为大端，且大于 0（gzip 压缩后）
        let len = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize;
        assert_eq!(len, frame.len() - 8);
        assert!(len > 0);
    }

    #[test]
    fn test_parse_audio_response_last_frame() {
        // 构造 audio-only 帧：头(0x11,0xBB,0x11,0x00) + [seq=4B][size=4B] + 音频数据
        let mut frame = vec![0x11, 0xBB, 0x11, 0x00];
        frame.extend_from_slice(&(-1i32).to_be_bytes()); // seq < 0 => 最后一包
        frame.extend_from_slice(&3u32.to_be_bytes());
        frame.extend_from_slice(&[1, 2, 3]); // 音频
        let (audio, is_last) = parse_audio_response(&frame, DEFAULT_RESOURCE_ID).unwrap();
        assert!(is_last);
        assert_eq!(audio, vec![1, 2, 3]);
    }

    #[test]
    fn test_parse_audio_response_mid_frame() {
        let mut frame = vec![0x11, 0xBB, 0x11, 0x00];
        frame.extend_from_slice(&5i32.to_be_bytes()); // seq > 0
        frame.extend_from_slice(&2u32.to_be_bytes());
        frame.extend_from_slice(&[9, 8]);
        let (audio, is_last) = parse_audio_response(&frame, DEFAULT_RESOURCE_ID).unwrap();
        assert!(!is_last);
        assert_eq!(audio, vec![9, 8]);
    }

    #[test]
    fn test_parse_error_response() {
        let mut frame = vec![0x11, 0xF0, 0x11, 0x00];
        frame.extend_from_slice(&90001i32.to_be_bytes());
        frame.extend_from_slice(&4u32.to_be_bytes());
        frame.extend_from_slice(b"oops");
        let err = parse_audio_response(&frame, DEFAULT_RESOURCE_ID).unwrap_err();
        assert!(err.to_string().contains("90001"));
        // 错误里带上 resource id，便于下次定位
        assert!(err
            .to_string()
            .contains("resource_id=volc.service_type.10029"));
    }

    #[test]
    fn test_build_ws_request_has_required_headers() {
        let tts = sample_provider("v1");
        let req = tts.build_ws_request().expect("build request");
        let h = req.headers();
        let get = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);

        assert_eq!(get("X-Api-App-Id").as_deref(), Some("1234567890"));
        assert_eq!(
            get("X-Api-Access-Key").as_deref(),
            Some("test-access-key-value")
        );
        assert_eq!(
            get("X-Api-Resource-Id").as_deref(),
            Some("volc.service_type.10029")
        );
        assert!(h.contains_key("X-Api-App-Key"));
        assert!(h.contains_key("X-Api-Connect-Id"));
        assert!(get("Authorization")
            .map(|v| v.starts_with("Bearer;"))
            .unwrap_or(false));
    }

    #[test]
    fn test_v3_selects_bidirection_endpoint() {
        let tts = sample_provider("v3");
        let req = tts.build_ws_request().unwrap();
        assert!(req.uri().to_string().contains("/api/v3/tts/bidirection"));
    }

    #[test]
    fn test_build_event_frame_start_connection() {
        // 连接级事件不带 session id：头 + event + payload 长度 + payload
        let frame = build_event_frame(EVENT_START_CONNECTION, "", b"{}");
        assert_eq!(&frame[0..4], &[0x11, 0x14, 0x10, 0x00]);
        let event = i32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]);
        assert_eq!(event, EVENT_START_CONNECTION);
        let plen = u32::from_be_bytes([frame[8], frame[9], frame[10], frame[11]]) as usize;
        assert_eq!(plen, 2);
        assert_eq!(&frame[12..], b"{}");
    }

    #[test]
    fn test_build_event_frame_with_session_id() {
        // 会话级事件带 session id
        let frame = build_event_frame(EVENT_TASK_REQUEST, "sid-123", b"hi");
        let event = i32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]);
        assert_eq!(event, EVENT_TASK_REQUEST);
        let sid_len = u32::from_be_bytes([frame[8], frame[9], frame[10], frame[11]]) as usize;
        assert_eq!(sid_len, 7);
        assert_eq!(&frame[12..12 + sid_len], b"sid-123");
        let plen = u32::from_be_bytes([
            frame[12 + sid_len],
            frame[13 + sid_len],
            frame[14 + sid_len],
            frame[15 + sid_len],
        ]) as usize;
        assert_eq!(plen, 2);
        assert_eq!(&frame[16 + sid_len..], b"hi");
    }
}
