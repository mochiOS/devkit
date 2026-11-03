//! Disposable SDK-format fixtures validate the manager, not compiler or mochiOS runtime compatibility.
use super::*;
use flate2::{write::GzEncoder, Compression};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;

fn fixture(directory: &Path, version: &str) -> PathBuf {
    let source = directory.join(format!("source-{version}"));
    fs::create_dir_all(&source).unwrap();
    let name = format!("x86_64-mochios-sdk-{version}.tar.gz");
    let sdk = json!({"format":1,"version":version,"target":TARGET,"abi":ABI,"host":HOST,
        "features":["pic","shared","cxx23","threads","exceptions"],"sysroot":"sysroot","cmake_toolchain":"share/cmake/mochios.cmake"});
    let files = vec![
        ("sdk.json", serde_json::to_vec(&sdk).unwrap(), 0o644),
        (
            "bin/driver.py",
            b"#!/usr/bin/env python3\n# fixture only; never execute\n".to_vec(),
            0o755,
        ),
        (
            "host/bin/clang",
            b"fixture compiler; never execute\n".to_vec(),
            0o755,
        ),
        (
            "host/bin/ld.lld",
            b"fixture linker; never execute\n".to_vec(),
            0o755,
        ),
        (
            "host/bin/ar",
            b"fixture archive tool; never execute\n".to_vec(),
            0o755,
        ),
        (
            "host/bin/ranlib",
            b"fixture archive tool; never execute\n".to_vec(),
            0o755,
        ),
        (
            "share/cmake/mochios.cmake",
            b"# fixture only\n".to_vec(),
            0o644,
        ),
        (
            "sysroot/include/fixture.h",
            b"/* fixture only */\n".to_vec(),
            0o644,
        ),
    ];
    let archive = fs::File::create(source.join(&name)).unwrap();
    let mut tar = tar::Builder::new(GzEncoder::new(archive, Compression::fast()));
    let mut inventory = serde_json::Map::new();
    for (path, data, mode) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        tar.append_data(&mut header, format!("sdk/{path}"), Cursor::new(&data))
            .unwrap();
        inventory.insert(
            path.into(),
            json!({"size":data.len(),"sha256":format!("{:x}",Sha256::digest(&data))}),
        );
    }
    for (path, target) in [
        ("bin/mochios-cc", "driver.py"),
        ("bin/mochios-c++", "driver.py"),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name(target).unwrap();
        header.set_cksum();
        tar.append_data(&mut header, format!("sdk/{path}"), Cursor::new([]))
            .unwrap();
        inventory.insert(path.into(), json!({"symlink":target}));
    }
    tar.into_inner().unwrap().finish().unwrap();
    let hash = format!(
        "{:x}",
        Sha256::digest(fs::read(source.join(&name)).unwrap())
    );
    fs::write(source.join("SHA256SUMS"), format!("{hash}  {name}\n")).unwrap();
    let manifest = json!({"format":1,"sdk":sdk,"archive":name,"sha256":hash,"files":inventory});
    fs::write(
        source.join(format!("{name}.json")),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    source
}

fn unpack_raw(
    root: &Path,
    entry_name: &str,
    kind: tar::EntryType,
    target: Option<&str>,
    mode: u32,
) -> anyhow::Result<()> {
    let file = root.join("test.tar.gz");
    let mut tar = tar::Builder::new(GzEncoder::new(
        fs::File::create(&file)?,
        Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_mode(mode);
    header.set_size(0);
    // Deliberately bypass the writer's path sanitization to exercise hostile archive input.
    let name = entry_name.as_bytes();
    header.as_mut_bytes()[..name.len()].copy_from_slice(name);
    if let Some(link) = target {
        header.set_link_name(link)?;
    }
    header.set_cksum();
    tar.append(&header, Cursor::new([]))?;
    tar.into_inner()?.finish()?;
    let text = entry_name.strip_prefix("sdk/").unwrap_or(entry_name);
    let record = if let Some(link) = target {
        json!({"symlink":link})
    } else {
        json!({"size":0,"sha256":format!("{:x}",Sha256::digest([]))})
    };
    let manifest: Manifest = serde_json::from_value(
        json!({"format":1,"sdk":{},"archive":"test.tar.gz","sha256":"0".repeat(64),"files":{text:record}}),
    )?;
    let output = root.join("payload");
    fs::create_dir(&output)?;
    archive::extract(&file, &output, &manifest)
}

#[test]
fn path_and_version_validation_reject_escape_and_control_characters() {
    for value in [
        "../other",
        "/root",
        "x/../other",
        "x//other",
        "x/./other",
        "x\\other",
        "C:/path",
        "line\nbreak",
        "",
    ] {
        assert!(relative(value).is_err(), "{value}");
    }
    for value in ["../other", "27/one", "..", "a\nb", ""] {
        assert!(version_check(value).is_err(), "{value}");
    }
    assert!(version_check("27.0-dp.1-shared.2").is_ok());
    assert!(relative("share/cmake/mochios.cmake").is_ok());
}

#[test]
fn unsafe_archive_entries_never_escape_private_staging() {
    for (name, kind, target, mode) in [
        ("sdk/../escaped", tar::EntryType::Regular, None, 0o644),
        ("/absolute", tar::EntryType::Regular, None, 0o644),
        (
            "sdk/bin/link",
            tar::EntryType::Symlink,
            Some("../../outside"),
            0o777,
        ),
        ("sdk/bin/privileged", tar::EntryType::Regular, None, 0o4755),
        (
            "sdk/hardlink",
            tar::EntryType::Link,
            Some("sdk/other"),
            0o644,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        assert!(
            unpack_raw(root.path(), name, kind, target, mode).is_err(),
            "{name}"
        );
        assert!(!root.path().join("escaped").exists());
        assert!(!root.path().join("outside").exists());
    }
}

#[test]
fn versions_switch_atomically_without_touching_the_kome_toolchain() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home with spaces");
    fs::create_dir(&home).unwrap();
    fs::write(
        home.join("komeup.toml"),
        b"original Kome toolchain configuration",
    )
    .unwrap();
    fs::create_dir(home.join("bin")).unwrap();
    fs::write(home.join("bin/kome"), b"original Kome command").unwrap();
    let store = store::Store::open(home.clone()).unwrap();
    let one = fixture(root.path(), "27.0-dp.1-shared.1");
    let two = fixture(root.path(), "27.0-dp.1-shared.2");
    install(&store, "27.0-dp.1-shared.1", Some(&one), false).unwrap();
    let pinned = store.selected(Some("27.0-dp.1-shared.1")).unwrap().0;
    install(&store, "27.0-dp.1-shared.2", Some(&two), false).unwrap();
    assert_eq!(
        store.current().unwrap().as_deref(),
        Some("27.0-dp.1-shared.1")
    );
    store.activate("27.0-dp.1-shared.2").unwrap();
    assert_eq!(
        store.current().unwrap().as_deref(),
        Some("27.0-dp.1-shared.2")
    );
    assert!(pinned.join("share/cmake/mochios.cmake").is_file());
    assert_eq!(
        fs::read(home.join("komeup.toml")).unwrap(),
        b"original Kome toolchain configuration"
    );
    assert_eq!(
        fs::read(home.join("bin/kome")).unwrap(),
        b"original Kome command"
    );
    store.uninstall("27.0-dp.1-shared.2").unwrap();
    assert!(store.current().unwrap().is_none());
    assert!(!home.join("bin/mochios-cc").is_symlink());
    store.activate("27.0-dp.1-shared.1").unwrap();
    assert_eq!(store.selected(None).unwrap().0, pinned);
}

#[test]
fn a_bad_download_preserves_the_previous_default() {
    let root = tempfile::tempdir().unwrap();
    let store = store::Store::open(root.path().join("home")).unwrap();
    let one = fixture(root.path(), "27.0-dp.1-shared.1");
    install(&store, "27.0-dp.1-shared.1", Some(&one), true).unwrap();
    let two = fixture(root.path(), "27.0-dp.1-shared.2");
    fs::write(
        two.join("x86_64-mochios-sdk-27.0-dp.1-shared.2.tar.gz"),
        b"damaged archive",
    )
    .unwrap();
    assert!(install(&store, "27.0-dp.1-shared.2", Some(&two), true).is_err());
    assert_eq!(
        store.current().unwrap().as_deref(),
        Some("27.0-dp.1-shared.1")
    );
    assert!(!store
        .home
        .join("sdks/installed/27.0-dp.1-shared.2")
        .exists());
    assert_eq!(
        fs::read_dir(store.home.join("sdks/staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn same_version_with_different_bytes_and_unmanaged_shims_are_not_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let store = store::Store::open(root.path().join("home")).unwrap();
    let one = fixture(root.path(), "27.0-dp.1-shared.1");
    install(&store, "27.0-dp.1-shared.1", Some(&one), false).unwrap();
    let receipt = store::Receipt::new("27.0-dp.1-shared.1", "local", &"a".repeat(64)).unwrap();
    let payload = root.path().join("unused");
    fs::create_dir(&payload).unwrap();
    assert!(store.commit(&payload, b"{}", &receipt).is_err());
    let shim = store.home.join("bin/mochios-cc");
    fs::remove_file(&shim).unwrap();
    fs::write(&shim, b"user command").unwrap();
    assert!(store.activate("27.0-dp.1-shared.1").is_err());
    assert_eq!(fs::read(&shim).unwrap(), b"user command");
}

#[test]
fn installed_file_changes_are_detected_without_running_sdk_programs() {
    let root = tempfile::tempdir().unwrap();
    let store = store::Store::open(root.path().join("home")).unwrap();
    let one = fixture(root.path(), "27.0-dp.1-shared.1");
    install(&store, "27.0-dp.1-shared.1", Some(&one), false).unwrap();
    let (installed, _) = store.selected(None).unwrap();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(installed.join(INSTALLED_MANIFEST)).unwrap()).unwrap();
    fs::write(installed.join("host/bin/clang"), b"changed").unwrap();
    assert!(archive::verify_installed(&installed, &manifest).is_err());
}

#[test]
fn sdk_cli_does_not_steal_the_root_version_flag_or_edit_project_configuration() {
    use clap::Parser;
    let parsed = crate::cli::Cli::try_parse_from([
        "komeup",
        "sdk",
        "path",
        "--version",
        "27.0-dp.1-shared.2",
        "--cmake",
    ])
    .unwrap();
    assert!(matches!(
        parsed.command,
        crate::cli::Command::Sdk {
            command: SdkCommand::Path { cmake: true, .. }
        }
    ));
    assert!(
        crate::cli::Cli::try_parse_from(["komeup", "sdk", "path", "--bin", "--cmake"]).is_err()
    );
}

#[test]
fn a_damaged_installed_sdk_cannot_replace_a_working_default() {
    let root = tempfile::tempdir().unwrap();
    let store = store::Store::open(root.path().join("home")).unwrap();
    let one = fixture(root.path(), "27.0-dp.1-shared.1");
    let two = fixture(root.path(), "27.0-dp.1-shared.2");
    install(&store, "27.0-dp.1-shared.1", Some(&one), false).unwrap();
    install(&store, "27.0-dp.1-shared.2", Some(&two), false).unwrap();
    let (second, _) = store.selected(Some("27.0-dp.1-shared.2")).unwrap();
    fs::write(second.join("host/bin/clang"), b"tampered compiler").unwrap();
    assert!(store.activate("27.0-dp.1-shared.2").is_err());
    assert_eq!(
        store.current().unwrap().as_deref(),
        Some("27.0-dp.1-shared.1")
    );
}

#[test]
fn an_explicit_version_does_not_depend_on_the_default_selector() {
    let root = tempfile::tempdir().unwrap();
    let store = store::Store::open(root.path().join("home")).unwrap();
    let one = fixture(root.path(), "27.0-dp.1-shared.1");
    install(&store, "27.0-dp.1-shared.1", Some(&one), false).unwrap();
    let selector = store.home.join("sdks/current");
    fs::remove_file(&selector).unwrap();
    fs::write(&selector, b"unmanaged selector").unwrap();
    assert!(store.selected(None).is_err());
    assert!(store.selected(Some("27.0-dp.1-shared.1")).is_ok());
    assert!(store.activate("27.0-dp.1-shared.1").is_err());
    assert_eq!(fs::read(selector).unwrap(), b"unmanaged selector");
}

#[test]
fn duplicate_entries_and_symlink_parent_writes_are_rejected() {
    for entries in [
        vec![("same", None), ("same", None)],
        vec![("redirect", Some("directory")), ("redirect/file", None)],
        vec![("cycle", Some("cycle"))],
    ] {
        let root = tempfile::tempdir().unwrap();
        let archive_path = root.path().join("test.tar.gz");
        let mut tar = tar::Builder::new(GzEncoder::new(
            fs::File::create(&archive_path).unwrap(),
            Compression::fast(),
        ));
        let mut inventory = serde_json::Map::new();
        for (path, target) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o755);
            let record = if let Some(link) = target {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name(link).unwrap();
                json!({"symlink":link})
            } else {
                json!({"size":0,"sha256":format!("{:x}",Sha256::digest([]))})
            };
            header.set_cksum();
            tar.append_data(&mut header, format!("sdk/{path}"), Cursor::new([]))
                .unwrap();
            inventory.insert(path.into(), record);
        }
        tar.into_inner().unwrap().finish().unwrap();
        let manifest: Manifest = serde_json::from_value(json!({
            "format":1,"sdk":{},"archive":"test.tar.gz","sha256":"0".repeat(64),"files":inventory
        }))
        .unwrap();
        let payload = root.path().join("payload");
        fs::create_dir(&payload).unwrap();
        assert!(archive::extract(&archive_path, &payload, &manifest).is_err());
        assert!(!root.path().join("directory/file").exists());
    }
}

#[test]
fn simultaneous_first_installs_keep_one_valid_default_and_both_versions() {
    use std::sync::{Arc, Barrier};
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let gate = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for version in ["27.0-dp.1-shared.1", "27.0-dp.1-shared.2"] {
        let source = fixture(root.path(), version);
        let store = store::Store::open(home.clone()).unwrap();
        let gate = gate.clone();
        workers.push(std::thread::spawn(move || {
            gate.wait();
            install(&store, version, Some(&source), false)
        }));
    }
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    let store = store::Store::open(home).unwrap();
    assert!(store.selected(None).is_ok());
    assert!(store.selected(Some("27.0-dp.1-shared.1")).is_ok());
    assert!(store.selected(Some("27.0-dp.1-shared.2")).is_ok());
}
