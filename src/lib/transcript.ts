// 转录稿解析：与后端 queue::pipeline::split_transcript 的识别规则保持一致。
// 抽成纯函数模块，便于脱离 Svelte 组件直接验证（见 /tmp 的一次性对拍脚本）。

export type Speaker = "PERSON_1" | "PERSON_2" | null;

export interface TranscriptRow {
  speaker: Speaker;
  text: string;
}

export interface ParsedTranscript {
  rows: TranscriptRow[];
  p1: number;
  p2: number;
  /** 可被合成语音的正文总字数（不含说话人前缀）。 */
  chars: number;
  /** 缺前缀或正文为空、后端会跳过的行数。 */
  ignored: number;
}

/** 识别一行的说话人；无法识别返回 null。 */
export function speakerOf(line: string): Speaker {
  const idx = line.indexOf(":");
  if (idx === -1) return null;
  const head = line.slice(0, idx).trim();
  if (head === "PERSON_1" || head === "PERSON_2") return head;
  // 后端兜底：首字母 P/A/Q 且含 "PERSON_x:" 时按小写比对。
  if (/^[PAQ]/.test(line) && (line.includes("PERSON_1:") || line.includes("PERSON_2:"))) {
    const low = head.toLowerCase();
    if (low === "person_1") return "PERSON_1";
    if (low === "person_2") return "PERSON_2";
  }
  return null;
}

export function parseTranscript(raw: string): ParsedTranscript {
  const rows: TranscriptRow[] = [];
  let p1 = 0;
  let p2 = 0;
  let chars = 0;
  let ignored = 0;
  for (const rawLine of raw.split("\n")) {
    const line = rawLine.trim();
    if (!line) continue;
    const speaker = speakerOf(line);
    const text = speaker ? line.slice(line.indexOf(":") + 1).trim() : line;
    if (!speaker || !text) {
      rows.push({ speaker: null, text: line });
      ignored += 1;
      continue;
    }
    rows.push({ speaker, text });
    chars += text.length;
    if (speaker === "PERSON_1") p1 += 1;
    else p2 += 1;
  }
  return { rows, p1, p2, chars, ignored };
}
