#![deny(missing_docs)]
//! アプリが提供する動的サムネイル。ファイル名やパスは生成担当へ渡しません。
//! 入力32MiB、出力1024px以内のpremultiplied RGBA8、期限3秒。永続キャッシュは作りません。
use crate::worker::{self, WorkerLimits};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    time::Duration,
};
const MAGIC: &[u8; 8] = b"MAPPTH01";
/// ワーカー起動時にAppCoreが使用する引数。アプリはUI起動前にserveを呼びます。
pub const WORKER_ARGUMENT: &str = "--appcore-thumbnail-v1";
/// 入力データの最大サイズ。
pub const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
/// 出力の幅・高さの上限。
pub const MAX_EDGE: u32 = 1024;

/// 検証済みサイズのサムネイル画像。
#[derive(Clone, Debug)]
pub struct Thumbnail {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl Thumbnail {
    /// premultiplied RGBA8から作成します。不正寸法・長さ・アルファ値は拒否します。
    pub fn from_premultiplied_rgba8(width: u32, height: u32, pixels: Vec<u8>) -> io::Result<Self> {
        if width == 0
            || height == 0
            || width > MAX_EDGE
            || height > MAX_EDGE
            || pixels.len() != width as usize * height as usize * 4
            || pixels
                .chunks_exact(4)
                .any(|pixel| pixel[..3].iter().any(|channel| *channel > pixel[3]))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid thumbnail pixels",
            ));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    /// ピクセル単位の幅。
    pub fn width(&self) -> u32 {
        self.width
    }
    /// ピクセル単位の高さ。
    pub fn height(&self) -> u32 {
        self.height
    }
    /// 行順のpremultiplied RGBA8画像データ。
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}
/// 画像生成へ渡す入力。パスを解決する権限は含みません。
pub struct ThumbnailRequest {
    /// 元ファイルのバイト列。
    pub bytes: Vec<u8>,
    /// 関連付けに使用した内容型。
    pub content_type: String,
    /// ドットを含まない拡張子。
    pub extension: String,
    /// 縦横を収める最大ピクセル数。
    pub max_edge: u32,
}
/// manifestから解決された、明示的なサムネイル生成担当。
#[derive(Clone, Debug)]
pub struct ThumbnailProvider {
    /// 生成担当アプリの識別子。
    pub bundle_id: String,
    entry: PathBuf,
}
impl ThumbnailProvider {
    pub(super) fn new(bundle_id: String, entry: PathBuf) -> Self {
        Self { bundle_id, entry }
    }
    /// 生成担当の実行ファイル。キャッシュ無効化などの更新確認に使えます。
    pub fn executable(&self) -> &std::path::Path {
        &self.entry
    }
    /// 別プロセスで生成し、寸法・データ長・アルファ・終了結果を検証します。
    /// UIスレッドから呼ばないでください。エラー時は静的アイコンへ戻してください。
    /// mochiOSでは `process.spawn` と既存アプリ起動ポリシーが適用されます。
    /// `cancelled` がtrueを返すと子を停止します。入力上限32MiB、期限3秒です。
    pub fn generate(
        &self,
        bytes: &[u8],
        extension: &str,
        content_type: &str,
        max_edge: u32,
        cancelled: impl Fn() -> bool,
    ) -> io::Result<Thumbnail> {
        validate_request(extension, content_type, max_edge)?;
        let edge = max_edge.to_string();
        let response = worker::run(
            &self.entry,
            &[WORKER_ARGUMENT, content_type, extension, &edge],
            bytes,
            WorkerLimits {
                output_bytes: 16 + max_edge as usize * max_edge as usize * 4,
                timeout: Duration::from_secs(3),
            },
            cancelled,
        )?;
        decode_response(&response, max_edge)
    }
}
fn validate_request(extension: &str, content_type: &str, edge: u32) -> io::Result<()> {
    if edge == 0
        || edge > MAX_EDGE
        || extension.is_empty()
        || extension.len() > 63
        || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        || crate::content_type::ContentType::parse(content_type).is_err()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid thumbnail request",
        ));
    }
    Ok(())
}
fn decode_response(bytes: &[u8], edge: u32) -> io::Result<Thumbnail> {
    if bytes.len() < 16 || &bytes[..8] != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid thumbnail response",
        ));
    }
    let width = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let height = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    if width > edge || height > edge {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oversized thumbnail response",
        ));
    }
    Thumbnail::from_premultiplied_rgba8(width, height, bytes[16..].to_vec())
}
/// サムネイル起動要求だけを処理します。通常起動では `Ok(false)` を返します。
/// アプリのmainで、ウィンドウやGPUを初期化する前に呼んでください。
/// 生成担当は内容型・拡張子を検証し、対応しない入力をエラーにしてください。
/// ## Errors
/// 不正な要求、入力上限超過、生成失敗、標準入出力エラーを返します。
/// ワーカーモードのエラーではUIへ進まず、非0で終了してください。
pub fn serve(generate: impl FnOnce(ThumbnailRequest) -> io::Result<Thumbnail>) -> io::Result<bool> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) != Some(WORKER_ARGUMENT) {
        return Ok(false);
    }
    if arguments.len() != 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid worker arguments",
        ));
    }
    let max_edge = arguments[3]
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid thumbnail size"))?;
    validate_request(&arguments[2], &arguments[1], max_edge)?;
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "thumbnail input too large",
        ));
    }
    let thumbnail = generate(ThumbnailRequest {
        bytes,
        content_type: arguments[1].clone(),
        extension: arguments[2].clone(),
        max_edge,
    })?;
    if thumbnail.width > max_edge || thumbnail.height > max_edge {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "thumbnail output too large",
        ));
    }
    let mut output = io::stdout().lock();
    output.write_all(MAGIC)?;
    output.write_all(&thumbnail.width.to_le_bytes())?;
    output.write_all(&thumbnail.height.to_le_bytes())?;
    output.write_all(&thumbnail.pixels)?;
    output.flush()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_malformed_oversized_and_nonpremultiplied_responses() {
        assert!(decode_response(&[], 144).is_err());
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[128, 0, 0, 128]);
        let image = decode_response(&bytes, 144).unwrap();
        assert_eq!((image.width(), image.height()), (1, 1));
        assert!(decode_response(&bytes, 0).is_err());
        bytes.push(0);
        assert!(decode_response(&bytes, 144).is_err());
        bytes.pop();
        bytes[16] = 255;
        assert!(decode_response(&bytes, 144).is_err());
        bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_response(&bytes, 144).is_err());
        assert!(Thumbnail::from_premultiplied_rgba8(0, 1, vec![]).is_err());
    }
    #[test]
    fn validates_requested_dimensions_and_selectors() {
        assert!(validate_request("png", "image/png", 144).is_ok());
        for edge in [0, MAX_EDGE + 1, u32::MAX] {
            assert!(validate_request("png", "image/png", edge).is_err());
        }
        assert!(validate_request("../png", "image/png", 144).is_err());
        assert!(validate_request("png", "not mime", 144).is_err());
    }
}
