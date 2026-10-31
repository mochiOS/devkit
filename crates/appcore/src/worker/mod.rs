#![deny(missing_docs)]
//! ウィンドウを開かないアプリ処理との、上限・期限付き標準入出力通信。
//! mochiOSでは既存の署名・Capability検証付き起動機構を使用します。
//! 呼出元には `process.spawn` と実行ファイルを読み取る権限が必要です。
//! 子に呼出元のCapabilityを継承させず、OSのアプリ起動ポリシーに従います。
//! Linuxプレビューでは通常の子プロセスとして起動します。mochiOSの隔離を再現するものではありません。
use std::{
    fs::File,
    io::{self, Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    path::Path,
    time::{Duration, Instant},
};
#[cfg(target_os = "mochios")]
mod native;

/// アプリワーカーの最大出力量と実行期限。
#[derive(Clone, Copy, Debug)]
pub struct WorkerLimits {
    /// 標準出力から受け付ける最大バイト数。最大16MiBです。
    pub output_bytes: usize,
    /// 起動後のI/Oと終了を待つ時間。0より大きく、最大60秒です。
    pub timeout: Duration,
}

fn pipe() -> io::Result<(File, File)> {
    let mut fds = [-1; 2];
    // SAFETY: two valid integer slots are supplied; ownership is adopted once.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let pair = unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
    for file in [&pair.0, &pair.1] {
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(pair)
}
fn nonblocking(file: &File) -> io::Result<()> {
    let fd = file.as_raw_fd();
    // SAFETY: borrowed open descriptor, flags only; no ownership transfer.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
struct Child {
    #[cfg(target_os = "linux")]
    process: std::process::Child,
    #[cfg(target_os = "mochios")]
    pid: libc::pid_t,
    reaped: bool,
}
impl Child {
    fn start(path: &Path, args: &[&str], stdio: [File; 3]) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let [input, output, error] = stdio;
            let process = std::process::Command::new(path)
                .args(args)
                .env_remove("VIEWKIT_PREVIEW_SOCKET")
                .env_remove("VIEWKIT_PREVIEW_CAPTURE")
                .stdin(input)
                .stdout(output)
                .stderr(error)
                .spawn()?;
            Ok(Self {
                process,
                reaped: false,
            })
        }
        #[cfg(target_os = "mochios")]
        {
            let path = path.to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 application path")
            })?;
            let mut arguments = vec![path];
            arguments.extend_from_slice(args);
            let pid = native::spawn(path, &arguments, stdio.each_ref().map(AsRawFd::as_raw_fd))?;
            Ok(Self { pid, reaped: false })
        }
    }
    fn status(&mut self) -> io::Result<Option<bool>> {
        #[cfg(target_os = "linux")]
        let status = self.process.try_wait()?.map(|status| status.success());
        #[cfg(target_os = "mochios")]
        let status = {
            let mut status = 0;
            let result = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if result < 0 {
                return Err(io::Error::last_os_error());
            }
            if result == 0 {
                None
            } else {
                Some(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0)
            }
        };
        self.reaped |= status.is_some();
        Ok(status)
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        #[cfg(target_os = "linux")]
        {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
        #[cfg(target_os = "mochios")]
        unsafe {
            libc::kill(self.pid, libc::SIGKILL);
            libc::waitpid(self.pid, std::ptr::null_mut(), 0);
        }
    }
}

/// 検証済みアプリを起動し、入力を標準入力へ渡し、標準出力を返します。
///
/// UIスレッドから呼ばず、TaskScopeなどのバックグラウンド処理で使用してください。
/// 入出力は同時に進めるため、パイプの容量による相互待ちを防ぎます。
/// 追加スレッド・一時ファイル・シェル展開は使用しません。
/// キャンセル・期限切れ・異常終了・出力上限超過時には子を終了・回収します。
///
/// ## Parameters
/// - `path`: インストール済みアプリの実行ファイルの絶対パス。
/// - `arguments`: シェルを介さず渡す引数。
/// - `input`: 対象データ。最大32MiB。
/// - `limits`: 出力サイズと時間の上限。
/// - `cancelled`: 中止要求を確認する、短時間で返るコールバック。
/// ## Errors
/// 権限不足、起動・I/Oの失敗、不正な制限、中止、期限切れを返します。
/// 起動自体の同期OS要求には別の期限を設定できません。
pub fn run(
    path: &Path,
    arguments: &[&str],
    input: &[u8],
    limits: WorkerLimits,
    cancelled: impl Fn() -> bool,
) -> io::Result<Vec<u8>> {
    if !path.is_absolute()
        || input.len() > 32 * 1024 * 1024
        || limits.output_bytes == 0
        || limits.output_bytes > 16 * 1024 * 1024
        || limits.timeout.is_zero()
        || limits.timeout > Duration::from_secs(60)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid worker limits or path",
        ));
    }
    if cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "worker cancelled",
        ));
    }
    let (child_input, input_writer) = pipe()?;
    let (mut output, child_output) = pipe()?;
    let (mut errors, child_error) = pipe()?;
    nonblocking(&input_writer)?;
    nonblocking(&output)?;
    nonblocking(&errors)?;
    let mut child = Child::start(path, arguments, [child_input, child_output, child_error])?;
    let deadline = Instant::now() + limits.timeout;
    let mut writer = Some(input_writer);
    let mut written = 0;
    let mut bytes = Vec::new();
    let mut eof = false;
    let mut status = None;
    loop {
        if cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "worker cancelled",
            ));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "worker timed out"));
        }
        if let Some(stream) = &mut writer {
            match stream.write(&input[written..]) {
                Ok(0) if written < input.len() => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "worker input closed",
                    ));
                }
                Ok(count) => written += count,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
            if written == input.len() {
                writer = None;
            }
        }
        let mut buffer = [0; 16 * 1024];
        if !eof {
            match output.read(&mut buffer) {
                Ok(0) => eof = true,
                Ok(count) => {
                    if bytes.len() + count > limits.output_bytes {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "worker output exceeds limit",
                        ));
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        // Drain and discard diagnostics, without storing untrusted output.
        match errors.read(&mut buffer) {
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        if status.is_none() {
            status = child.status()?;
        }
        if let Some(success) = status {
            if !success {
                return Err(io::Error::other("application worker failed"));
            }
            if eof {
                if written != input.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "worker did not consume input",
                    ));
                }
                return Ok(bytes);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    fn limits(bytes: usize) -> WorkerLimits {
        WorkerLimits {
            output_bytes: bytes,
            timeout: Duration::from_secs(2),
        }
    }
    #[test]
    fn drains_input_and_output_concurrently_beyond_pipe_capacity() {
        let input = vec![42; 256 * 1024];
        assert_eq!(
            run(
                Path::new("/bin/cat"),
                &[],
                &input,
                limits(input.len()),
                || false
            )
            .unwrap(),
            input
        );
    }
    #[test]
    fn timeout_and_cancellation_reap_the_worker() {
        let started = Instant::now();
        let error = run(
            Path::new("/bin/sleep"),
            &["5"],
            &[],
            WorkerLimits {
                output_bytes: 16,
                timeout: Duration::from_millis(30),
            },
            || false,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        let count = std::cell::Cell::new(0);
        let error = run(Path::new("/bin/sleep"), &["5"], &[], limits(16), || {
            count.set(count.get() + 1);
            count.get() > 3
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }
    #[test]
    fn rejects_failed_and_oversized_workers() {
        assert!(run(Path::new("/bin/false"), &[], &[], limits(16), || false).is_err());
        let error = run(Path::new("/usr/bin/yes"), &[], &[], limits(16), || false).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(run(Path::new("relative"), &[], &[], limits(16), || false).is_err());
    }
}
