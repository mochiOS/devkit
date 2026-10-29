//! 所有するファイルを通して文書を読み書きする、形式に依存しない境界です。
#![deny(missing_docs)]
use super::DocumentMetadata;
use crate::document::OpenedDocument;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
};

enum Resource {
    File(File),
    Received(OpenedDocument),
}
/// 権限検査を経て開いた、単一ファイルの所有参照です。
///
/// Cloneは同じ資源を共有し、各読み書きを直列化します。パスは表示用です。
/// I/Oのたびにパスを開き直したり、周辺ディレクトリの権限を取得したりしません。
#[derive(Clone)]
pub struct DocumentFile {
    path: PathBuf,
    resource: Arc<Mutex<Resource>>,
    writable: bool,
}
impl DocumentFile {
    /// 現在のアプリの権限で通常ファイルを開きます。ワーカー上で呼んでください。
    ///
    /// 書き込みも許可されれば保存可能な参照を保持し、それ以外は読み取り専用で開きます。
    /// ## Errors
    /// パスが読めない、通常ファイルでない場合はI/Oエラーです。権限は追加しません。
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let (file, writable) = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => (file, true),
            Err(_) => (File::open(&path)?, false),
        };
        Self::from_file(path, file, writable)
    }
    /// 保存先を現在の権限で開き、存在しなければ作成します。既存内容はまだ変更しません。
    ///
    /// 上書き確認は呼び出し側のSavePanelが行います。ワーカー専用です。
    /// ## Errors
    /// 権限不足、通常ファイル以外、容量不足などをI/Oエラーで返します。
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .or_else(|_| {
                OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&path)
            })?;
        Self::from_file(path, file, true)
    }
    fn from_file(path: PathBuf, file: File, writable: bool) -> io::Result<Self> {
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "A document must be a regular file",
            ));
        }
        Ok(Self {
            path,
            resource: Arc::new(Mutex::new(Resource::File(file))),
            writable,
        })
    }
    /// IPCで受け取った資源を所有します。表示用パスを再オープンしません。
    ///
    /// 現行の委譲インターフェースでは読み取り専用として扱い、保存はSave Asを使います。
    pub fn from_received(document: OpenedDocument) -> Self {
        Self {
            path: document.path.clone(),
            resource: Arc::new(Mutex::new(Resource::Received(document))),
            writable: false,
        }
    }
    /// 表示用のパスを返します。この値だけでアクセス権限があることは意味しません。
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// この参照を使って直接保存できる場合にtrueを返します。
    pub fn is_writable(&self) -> bool {
        self.writable
    }
    /// 先頭から最大maximumバイトを読みます。ワーカー専用です。
    ///
    /// ## Errors
    /// 上限超過はFileTooLarge、読み取り・seekの失敗は元のI/Oエラーです。
    pub fn read_limited(&self, maximum: usize) -> io::Result<Vec<u8>> {
        let mut resource = self
            .resource
            .lock()
            .map_err(|_| io::Error::other("Document resource lock failed"))?;
        match &mut *resource {
            Resource::Received(document) => document.read_to_end_limited(maximum),
            Resource::File(file) => {
                file.seek(SeekFrom::Start(0))?;
                let mut bytes = Vec::new();
                let mut buffer = [0; 8192];
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 {
                        return Ok(bytes);
                    }
                    if count > maximum.saturating_sub(bytes.len()) {
                        return Err(io::Error::new(
                            io::ErrorKind::FileTooLarge,
                            "Document exceeds its size limit",
                        ));
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
            }
        }
    }
    /// 同じ開いたファイルの内容を置換し、長さを調整してsync_allを行います。
    ///
    /// ワーカー専用です。兄弟の一時ファイルは作成しません。原子的置換ではないため、
    /// 書き込み途中の失敗では内容が一部変わる可能性があります。
    /// ## Errors
    /// 読み取り専用はPermissionDenied、書き込み・truncate・flush失敗は元のI/Oエラーです。
    pub fn replace_contents(&self, bytes: &[u8]) -> io::Result<()> {
        if !self.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Use Save As for a read-only document",
            ));
        }
        let mut resource = self
            .resource
            .lock()
            .map_err(|_| io::Error::other("Document resource lock failed"))?;
        let Resource::File(file) = &mut *resource else {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Delegated document is read-only",
            ));
        };
        file.seek(SeekFrom::Start(0))?;
        file.write_all(bytes)?;
        file.set_len(bytes.len() as u64)?;
        file.sync_all()
    }
}

/// ワーカーで動く文書形式の読み書きです。UIのStateを保持しないでください。
pub trait DocumentFormat: Send + Sync + 'static {
    /// ワーカーとUI間で所有権を移動する内容です。
    type Content: Send + 'static;
    /// 検証・デコードした内容と表示用メタデータを返します。失敗時はUIを変更しません。
    fn read(&self, file: &DocumentFile) -> Result<(Self::Content, DocumentMetadata), String>;
    /// UIが取得済みのスナップショットを保存します。書き込み・永続化失敗はErrにします。
    fn write(
        &self,
        file: &DocumentFile,
        content: Self::Content,
    ) -> Result<DocumentMetadata, String>;
}
/// 形式のI/OとUIのスナップショット取得・読み込み反映をつなぎます。
pub struct DocumentIo<F: DocumentFormat> {
    pub(super) format: Arc<F>,
    pub(super) snapshot: Rc<dyn Fn() -> Result<F::Content, String>>,
    pub(super) apply: Rc<dyn Fn(F::Content)>,
}
impl<F: DocumentFormat> DocumentIo<F> {
    /// 文書形式とUI側の2つの処理を登録します。
    ///
    /// - `format`: ワーカーで実行する読み書きです。
    /// - `snapshot`: 保存受付時にUI上で内容を複製します。失敗はErrで返します。
    /// - `apply`: 読み込み成功後にUI上で内容を適用します。Scope破棄後は呼びません。
    pub fn new(
        format: F,
        snapshot: impl Fn() -> Result<F::Content, String> + 'static,
        apply: impl Fn(F::Content) + 'static,
    ) -> Self {
        Self {
            format: Arc::new(format),
            snapshot: Rc::new(snapshot),
            apply: Rc::new(apply),
        }
    }
}
