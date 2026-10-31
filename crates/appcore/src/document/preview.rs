#![deny(missing_docs)]
//! アプリによる読み取り専用プレビュー生成。入力はストリームで渡し、パスは渡しません。
//! 登録はmanifestの `preview = true`。通常のアプリ起動・関連付けポリシーを使用します。
use super::thumbnail::{MAX_EDGE, MAX_INPUT_BYTES, Thumbnail};
use crate::worker::{self, WorkerLimits};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
const MAGIC: &[u8; 8] = b"MAPPPR01";
const ARGUMENT: &str = "--appcore-preview-v1";
/// テキストプレビューの最大UTF-8バイト数。大きな文書は生成側で切り詰めます。
pub const MAX_TEXT_BYTES: usize = 256 * 1024;
/// 生成担当から返される、実行可能なマークアップを含まない表示データ。
#[derive(Clone, Debug)]
pub enum PreviewContent {
    /// 寸法とpremultiplied alphaが検証済みの画像。
    Image(Thumbnail),
    /// UTF-8のプレーンテキスト。
    Text {
        /// 表示する本文。最大256KiB。
        text: String,
        /// 元文書の一部だけを返している場合はtrue。
        truncated: bool,
    },
}
/// 生成担当へ渡す入力。ファイルを開く追加権限は含みません。
pub struct PreviewRequest {
    /// 元データ。最大32MiB。
    pub bytes: Vec<u8>,
    /// 関連付けに使った内容型。
    pub content_type: String,
    /// ドットなしの拡張子。拡張子なしのファイルでは空文字列。
    pub extension: String,
    /// 出力画像の縦横を収める最大ピクセル数。
    pub max_edge: u32,
}
/// 関連付けとmanifestから解決されたプレビュー生成担当。
#[derive(Clone, Debug)]
pub struct PreviewProvider {
    /// 生成担当アプリの識別子。
    pub bundle_id: String,
    entry: PathBuf,
}
impl PreviewProvider {
    pub(super) fn new(bundle_id: String, entry: PathBuf) -> Self {
        Self { bundle_id, entry }
    }
    /// 生成担当の実行ファイル。
    pub fn executable(&self) -> &Path {
        &self.entry
    }
    /// 別プロセスでプレビューを生成します。UIスレッドから呼ばないでください。
    /// `bytes` は32MiB以下、`max_edge` は1〜1024。生成待ちの期限は3秒です。
    /// `cancelled` がtrueを返すと中止します。失敗時はエラーを返し、UIは開きません。
    /// mochiOSでは `process.spawn` と検証付きアプリ起動の権限判定を使用します。
    pub fn generate(
        &self,
        bytes: &[u8],
        extension: &str,
        content_type: &str,
        max_edge: u32,
        cancelled: impl Fn() -> bool,
    ) -> io::Result<PreviewContent> {
        validate(extension, content_type, max_edge)?;
        let edge = max_edge.to_string();
        let output = worker::run(
            &self.entry,
            &[ARGUMENT, content_type, extension, &edge],
            bytes,
            WorkerLimits {
                output_bytes: 24 + MAX_EDGE as usize * MAX_EDGE as usize * 4,
                timeout: Duration::from_secs(3),
            },
            cancelled,
        )?;
        decode(&output, max_edge)
    }
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid document preview")
}
fn validate(extension: &str, content_type: &str, edge: u32) -> io::Result<()> {
    if edge == 0
        || edge > MAX_EDGE
        || extension.len() > 63
        || !extension.bytes().all(|b| b.is_ascii_alphanumeric())
        || crate::content_type::ContentType::parse(content_type).is_err()
    {
        return Err(invalid());
    }
    Ok(())
}
fn encode(content: &PreviewContent, edge: u32) -> io::Result<Vec<u8>> {
    let mut bytes = MAGIC.to_vec();
    match content {
        PreviewContent::Image(image) => {
            if image.width() > edge || image.height() > edge {
                return Err(invalid());
            }
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&image.width().to_le_bytes());
            bytes.extend_from_slice(&image.height().to_le_bytes());
            bytes.extend_from_slice(image.pixels());
        }
        PreviewContent::Text { text, truncated } => {
            if text.len() > MAX_TEXT_BYTES || text.contains('\0') {
                return Err(invalid());
            }
            bytes.extend_from_slice(&2u32.to_le_bytes());
            bytes.extend_from_slice(&u32::from(*truncated).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
    }
    Ok(bytes)
}
fn decode(bytes: &[u8], edge: u32) -> io::Result<PreviewContent> {
    if bytes.len() < 16 || &bytes[..8] != MAGIC {
        return Err(invalid());
    }
    let kind = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let flags = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    match kind {
        1 if flags == 0 && bytes.len() >= 24 => {
            let width = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
            let height = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
            if width > edge || height > edge {
                return Err(invalid());
            }
            Ok(PreviewContent::Image(Thumbnail::from_premultiplied_rgba8(
                width,
                height,
                bytes[24..].to_vec(),
            )?))
        }
        2 if flags <= 1 && bytes.len() - 16 <= MAX_TEXT_BYTES => {
            let text = std::str::from_utf8(&bytes[16..]).map_err(|_| invalid())?;
            if text.contains('\0') {
                return Err(invalid());
            }
            Ok(PreviewContent::Text {
                text: text.into(),
                truncated: flags == 1,
            })
        }
        _ => Err(invalid()),
    }
}
/// アプリmainのUI初期化前に呼び、プレビュー要求だけを処理します。
/// 通常起動では `Ok(false)`、処理完了時は `Ok(true)` を返します。
/// エラー時にはUIへ進まず、プロセスを非0で終了してください。
/// 入力・出力の上限、不正な引数、生成失敗、標準入出力エラーを検証します。
pub fn serve(
    generate: impl FnOnce(PreviewRequest) -> io::Result<PreviewContent>,
) -> io::Result<bool> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some(ARGUMENT) {
        return Ok(false);
    }
    if args.len() != 4 {
        return Err(invalid());
    }
    let edge = args[3].parse().map_err(|_| invalid())?;
    validate(&args[2], &args[1], edge)?;
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(invalid());
    }
    let content = generate(PreviewRequest {
        bytes,
        extension: args[2].clone(),
        content_type: args[1].clone(),
        max_edge: edge,
    })?;
    io::stdout().lock().write_all(&encode(&content, edge)?)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips_unicode_text_and_truncation() {
        let content = PreviewContent::Text {
            text: "Hello\nこんにちは".into(),
            truncated: true,
        };
        let PreviewContent::Text { text, truncated } =
            decode(&encode(&content, 1024).unwrap(), 1024).unwrap()
        else {
            panic!()
        };
        assert_eq!(text, "Hello\nこんにちは");
        assert!(truncated);
    }
    #[test]
    fn validates_image_pixels_and_dimensions() {
        let content =
            PreviewContent::Image(Thumbnail::from_premultiplied_rgba8(2, 1, vec![128; 8]).unwrap());
        let mut bytes = encode(&content, 1024).unwrap();
        assert!(matches!(
            decode(&bytes, 1024).unwrap(),
            PreviewContent::Image(_)
        ));
        assert!(decode(&bytes, 1).is_err());
        bytes.push(0);
        assert!(decode(&bytes, 1024).is_err());
    }
    #[test]
    fn rejects_unknown_flags_utf8_nul_and_oversized_text() {
        let text = PreviewContent::Text {
            text: "text".into(),
            truncated: false,
        };
        let mut bytes = encode(&text, 1024).unwrap();
        bytes[12] = 2;
        assert!(decode(&bytes, 1024).is_err());
        bytes[12] = 0;
        bytes[16] = 255;
        assert!(decode(&bytes, 1024).is_err());
        bytes[16] = 0;
        assert!(decode(&bytes, 1024).is_err());
        assert!(
            encode(
                &PreviewContent::Text {
                    text: "a".repeat(MAX_TEXT_BYTES + 1),
                    truncated: false
                },
                1024
            )
            .is_err()
        );
    }
}
