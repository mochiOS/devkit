use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const ELF64_HEADER_LEN: usize = 64;
const ELF64_SECTION_HEADER_LEN: usize = 64;
const ELF64_PROGRAM_HEADER_LEN: usize = 56;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const SHT_STRTAB: u32 = 3;
const SHT_NOTE: u32 = 7;
const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
const SIGNATURE_SECTION: &[u8] = b".note.mochios.signature";
const NOTE_NAME: &[u8; 8] = b"mochiOS\0";
pub(crate) const NOTE_TYPE: u32 = 0x4d53_4947; // "MSIG"
const DESCRIPTOR_MAGIC: &[u8; 8] = b"MOELFSIG";
const VERSION: u16 = 1;
const KIND_ADHOC: u8 = 1;
const KIND_ED25519: u8 = 2;
const DIGEST_SHA256: u8 = 1;
const SIGNATURE_NONE: u8 = 0;
const SIGNATURE_ED25519: u8 = 1;
const FLAGS: u32 = 0;
const DESCRIPTOR_LEN: usize = 146;
const KEY_ID_OFFSET: usize = 18;
const DIGEST_OFFSET: usize = 50;
const SIGNATURE_OFFSET: usize = 82;
const SIGNING_CONTEXT: &[u8] = b"mochios-elf-signature-v1\0";
pub(crate) const MAX_ELF_LEN: usize = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignatureKind {
    AdHoc,
    Ed25519,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedSignature {
    pub kind: SignatureKind,
    pub key_id: Option<[u8; 32]>,
    pub digest: [u8; 32],
}

pub(crate) trait PublicKeyResolver {
    fn resolve(&self, key_id: &[u8; 32]) -> Result<Option<VerifyingKey>>;
}

#[derive(Clone, Copy)]
struct Section {
    name: u32,
    section_type: u32,
    offset: usize,
    size: usize,
    align: u64,
    entry_size: u64,
    raw_offset: usize,
}

struct Elf<'a> {
    bytes: &'a [u8],
    sections: Vec<Section>,
    shstr_index: usize,
    shstr: &'a [u8],
    program_notes: Vec<NoteArea>,
    load_segments: Vec<(usize, usize)>,
    program_header_table: Option<(usize, usize)>,
    section_header_table: Option<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoteOrigin {
    Section(usize),
    ProgramHeader(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NoteArea {
    offset: usize,
    size: usize,
    origin: NoteOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NoteOccurrence {
    descriptor_offset: usize,
    area: NoteArea,
}

#[derive(Clone)]
struct Record {
    kind: SignatureKind,
    digest_algorithm: u8,
    signature_algorithm: u8,
    flags: u32,
    key_id: [u8; 32],
    digest: [u8; 32],
    signature: [u8; 64],
    digest_file_offset: usize,
    signature_file_offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DecodedSignature {
    pub kind: SignatureKind,
    pub digest_algorithm: u8,
    pub signature_algorithm: u8,
    pub flags: u32,
    pub key_id: [u8; 32],
    pub digest: [u8; 32],
    pub signature: [u8; 64],
}

pub(crate) fn sign_adhoc(unsigned: &[u8]) -> Result<Vec<u8>> {
    let mut output = embed_empty_record(unsigned, SignatureKind::AdHoc, [0; 32])?;
    let record = locate_record(&output)?.context("ELF signature NOTE was not created")?;
    let digest = normalized_digest(&output, &record)?;
    output[record.digest_file_offset..record.digest_file_offset + 32].copy_from_slice(&digest);
    Ok(output)
}

pub(crate) fn sign_ed25519(unsigned: &[u8], key: &SigningKey) -> Result<Vec<u8>> {
    let public_key = key.verifying_key().to_bytes();
    let key_id: [u8; 32] = Sha256::digest(public_key).into();
    let mut output = embed_empty_record(unsigned, SignatureKind::Ed25519, key_id)?;
    let record = locate_record(&output)?.context("ELF signature NOTE was not created")?;
    let digest = normalized_digest(&output, &record)?;
    let message = signing_message(
        KIND_ED25519,
        DIGEST_SHA256,
        SIGNATURE_ED25519,
        FLAGS,
        &key_id,
        &digest,
    );
    let signature = key.sign(&message).to_bytes();
    output[record.digest_file_offset..record.digest_file_offset + 32].copy_from_slice(&digest);
    output[record.signature_file_offset..record.signature_file_offset + 64]
        .copy_from_slice(&signature);
    Ok(output)
}

pub(crate) fn verify(
    bytes: &[u8],
    resolver: Option<&dyn PublicKeyResolver>,
) -> Result<VerifiedSignature> {
    let record = decode_record(bytes)?;
    validate_record(&record)?;
    let actual = normalized_digest(bytes, &record)?;
    if actual != record.digest {
        bail!("ELF digest mismatch");
    }
    match record.kind {
        SignatureKind::AdHoc => Ok(VerifiedSignature {
            kind: SignatureKind::AdHoc,
            key_id: None,
            digest: actual,
        }),
        SignatureKind::Ed25519 => {
            let resolver = resolver.context("public key is required for keyed ELF signature")?;
            let public_key = resolver
                .resolve(&record.key_id)?
                .context("ELF signing key is unresolved")?;
            let actual_key_id: [u8; 32] = Sha256::digest(public_key.to_bytes()).into();
            if actual_key_id != record.key_id {
                bail!("public key does not match ELF key_id");
            }
            let message = signing_message(
                KIND_ED25519,
                record.digest_algorithm,
                record.signature_algorithm,
                record.flags,
                &record.key_id,
                &record.digest,
            );
            public_key
                .verify_strict(&message, &Signature::from_bytes(&record.signature))
                .context("invalid ELF Ed25519 signature")?;
            Ok(VerifiedSignature {
                kind: SignatureKind::Ed25519,
                key_id: Some(record.key_id),
                digest: actual,
            })
        }
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedSignature> {
    let record = decode_record(bytes)?;
    validate_record(&record)?;
    Ok(DecodedSignature {
        kind: record.kind,
        digest_algorithm: record.digest_algorithm,
        signature_algorithm: record.signature_algorithm,
        flags: record.flags,
        key_id: record.key_id,
        digest: record.digest,
        signature: record.signature,
    })
}

fn decode_record(bytes: &[u8]) -> Result<Record> {
    locate_record(bytes)?.context("ELF is not signed")
}

fn validate_record(record: &Record) -> Result<()> {
    if record.digest_algorithm != DIGEST_SHA256 {
        bail!("unsupported ELF digest algorithm");
    }
    if record.flags != 0 {
        bail!("unsupported ELF signature flags");
    }
    match record.kind {
        SignatureKind::AdHoc
            if record.signature_algorithm == SIGNATURE_NONE
                && record.key_id == [0; 32]
                && record.signature == [0; 64] =>
        {
            Ok(())
        }
        SignatureKind::AdHoc if record.signature_algorithm == SIGNATURE_NONE => {
            bail!("AdHoc signature contains key material")
        }
        SignatureKind::Ed25519 if record.signature_algorithm == SIGNATURE_ED25519 => Ok(()),
        _ => bail!("signature kind and algorithm do not match"),
    }
}

fn signing_message(
    kind: u8,
    digest_algorithm: u8,
    signature_algorithm: u8,
    flags: u32,
    key_id: &[u8; 32],
    digest: &[u8; 32],
) -> Vec<u8> {
    // Exact v1 message: context bytes including NUL, u8 kind, two u8 algorithm
    // identifiers (digest then signature), u32 little-endian flags, key_id, digest.
    let mut message = Vec::with_capacity(SIGNING_CONTEXT.len() + 71);
    message.extend_from_slice(SIGNING_CONTEXT);
    message.push(kind);
    message.push(digest_algorithm);
    message.push(signature_algorithm);
    message.extend_from_slice(&flags.to_le_bytes());
    message.extend_from_slice(key_id);
    message.extend_from_slice(digest);
    message
}

fn normalized_digest(bytes: &[u8], record: &Record) -> Result<[u8; 32]> {
    let digest_end = record
        .digest_file_offset
        .checked_add(32)
        .context("digest range overflow")?;
    let signature_end = record
        .signature_file_offset
        .checked_add(64)
        .context("signature range overflow")?;
    if digest_end > bytes.len()
        || signature_end > bytes.len()
        || digest_end > record.signature_file_offset
    {
        bail!("invalid ELF signature value ranges");
    }
    let mut hash = Sha256::new();
    hash.update(&bytes[..record.digest_file_offset]);
    hash.update([0u8; 32]);
    hash.update(&bytes[digest_end..record.signature_file_offset]);
    hash.update([0u8; 64]);
    hash.update(&bytes[signature_end..]);
    Ok(hash.finalize().into())
}

fn embed_empty_record(bytes: &[u8], kind: SignatureKind, key_id: [u8; 32]) -> Result<Vec<u8>> {
    let elf = parse_elf(bytes)?;
    if locate_record_in(&elf)?.is_some() {
        bail!("ELF already contains a signature NOTE");
    }
    if elf.sections.iter().any(
        |section| matches!(section_name(&elf, *section), Ok(name) if name == SIGNATURE_SECTION),
    ) {
        bail!("ELF already contains the reserved signature section");
    }
    let new_count = elf
        .sections
        .len()
        .checked_add(2)
        .context("section count overflow")?;
    let new_count_u16 = u16::try_from(new_count).context("ELF has too many sections")?;
    let old_shstr = elf.shstr;
    let mut shstr = Vec::new();
    shstr
        .try_reserve_exact(
            old_shstr
                .len()
                .checked_add(SIGNATURE_SECTION.len() + 2)
                .context("section string table size overflow")?,
        )
        .context("unable to allocate section string table")?;
    shstr.extend_from_slice(old_shstr);
    if shstr.last() != Some(&0) {
        shstr.push(0);
    }
    let signature_name = u32::try_from(shstr.len()).context("section string table is too large")?;
    shstr.extend_from_slice(SIGNATURE_SECTION);
    shstr.push(0);

    let mut descriptor = [0u8; DESCRIPTOR_LEN];
    descriptor[..8].copy_from_slice(DESCRIPTOR_MAGIC);
    descriptor[8..10].copy_from_slice(&VERSION.to_le_bytes());
    descriptor[10] = match kind {
        SignatureKind::AdHoc => KIND_ADHOC,
        SignatureKind::Ed25519 => KIND_ED25519,
    };
    descriptor[11] = DIGEST_SHA256;
    descriptor[12] = match kind {
        SignatureKind::AdHoc => SIGNATURE_NONE,
        SignatureKind::Ed25519 => SIGNATURE_ED25519,
    };
    descriptor[13] = 0;
    descriptor[14..18].copy_from_slice(&FLAGS.to_le_bytes());
    descriptor[KEY_ID_OFFSET..KEY_ID_OFFSET + 32].copy_from_slice(&key_id);

    let mut note = Vec::with_capacity(12 + NOTE_NAME.len() + align4(DESCRIPTOR_LEN)?);
    note.extend_from_slice(&(NOTE_NAME.len() as u32).to_le_bytes());
    note.extend_from_slice(&(DESCRIPTOR_LEN as u32).to_le_bytes());
    note.extend_from_slice(&NOTE_TYPE.to_le_bytes());
    note.extend_from_slice(NOTE_NAME);
    note.extend_from_slice(&descriptor);
    note.resize(align4(note.len())?, 0);

    let section_headers_size = new_count
        .checked_mul(ELF64_SECTION_HEADER_LEN)
        .context("section header table size overflow")?;
    let output_capacity = bytes
        .len()
        .checked_add(note.len())
        .and_then(|size| size.checked_add(shstr.len()))
        .and_then(|size| size.checked_add(section_headers_size))
        .and_then(|size| size.checked_add(16))
        .context("signed ELF size overflow")?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_capacity)
        .context("unable to allocate signed ELF")?;
    output.extend_from_slice(bytes);
    pad_to(&mut output, 4)?;
    let note_offset = output.len();
    output.extend_from_slice(&note);
    let shstr_offset = output.len();
    output.extend_from_slice(&shstr);
    pad_to(&mut output, 8)?;
    let section_table_offset = output.len();
    for section in &elf.sections {
        output.extend_from_slice(
            &bytes[section.raw_offset..section.raw_offset + ELF64_SECTION_HEADER_LEN],
        );
    }
    let mut note_header = [0u8; ELF64_SECTION_HEADER_LEN];
    put_u32(&mut note_header, 0, signature_name);
    put_u32(&mut note_header, 4, SHT_NOTE);
    put_u64(&mut note_header, 24, note_offset as u64);
    put_u64(&mut note_header, 32, note.len() as u64);
    put_u64(&mut note_header, 48, 4);
    output.extend_from_slice(&note_header);
    let old_shstr_section = elf.sections[elf.shstr_index];
    let mut shstr_header = [0u8; ELF64_SECTION_HEADER_LEN];
    put_u32(&mut shstr_header, 0, old_shstr_section.name);
    put_u32(&mut shstr_header, 4, SHT_STRTAB);
    put_u64(&mut shstr_header, 24, shstr_offset as u64);
    put_u64(&mut shstr_header, 32, shstr.len() as u64);
    put_u64(&mut shstr_header, 48, 1);
    output.extend_from_slice(&shstr_header);
    put_u64(&mut output, 40, section_table_offset as u64);
    put_u16(&mut output, 60, new_count_u16);
    let new_shstr_index = new_count_u16
        .checked_sub(1)
        .context("section string table index underflow")?;
    put_u16(&mut output, 62, new_shstr_index);
    // Reparse the completed ELF before any hash or signature is produced.
    let reparsed = parse_elf(&output)?;
    locate_record_in(&reparsed)?.context("generated ELF signature NOTE is not discoverable")?;
    Ok(output)
}

fn locate_record(bytes: &[u8]) -> Result<Option<Record>> {
    let elf = parse_elf(bytes)?;
    locate_record_in(&elf)
}

fn locate_record_in(elf: &Elf<'_>) -> Result<Option<Record>> {
    let mut note_areas = Vec::new();
    let mut signature_sections = Vec::new();
    for (index, section) in elf.sections.iter().enumerate() {
        if section_name(elf, *section)? == SIGNATURE_SECTION {
            if section.section_type != SHT_NOTE {
                bail!("reserved ELF signature section is not SHT_NOTE");
            }
            signature_sections.push(index);
        }
        if section.section_type == SHT_NOTE {
            note_areas.push(NoteArea {
                offset: section.offset,
                size: section.size,
                origin: NoteOrigin::Section(index),
            });
        }
    }
    if signature_sections.len() > 1 {
        bail!("ELF contains duplicate reserved signature sections");
    }
    note_areas.extend_from_slice(&elf.program_notes);

    let mut occurrences = Vec::new();
    for area in note_areas {
        parse_note_area(elf.bytes, area, &mut occurrences)?;
    }
    let occurrence = match occurrences.as_slice() {
        [] if signature_sections.is_empty() => return Ok(None),
        [] => bail!("reserved ELF signature section does not contain a signature NOTE"),
        [only] => *only,
        [first, second]
            if first.descriptor_offset == second.descriptor_offset
                && first.area.offset == second.area.offset
                && first.area.size == second.area.size
                && matches!(
                    (first.area.origin, second.area.origin),
                    (NoteOrigin::Section(_), NoteOrigin::ProgramHeader(_))
                        | (NoteOrigin::ProgramHeader(_), NoteOrigin::Section(_))
                ) =>
        {
            *first
        }
        _ => bail!("ELF contains duplicate or ambiguously referenced mochiOS signature NOTEs"),
    };
    let Some(signature_section_index) = signature_sections.into_iter().next() else {
        bail!("mochiOS signature NOTE is outside the reserved signature section");
    };
    let signature_section = elf.sections[signature_section_index];
    let section_end = signature_section.offset + signature_section.size;
    if occurrence.descriptor_offset < signature_section.offset
        || occurrence.descriptor_offset + DESCRIPTOR_LEN > section_end
    {
        bail!("mochiOS signature NOTE is outside the reserved signature section");
    }
    validate_signature_location(elf, occurrence)?;
    parse_record(elf.bytes, occurrence.descriptor_offset).map(Some)
}

fn parse_note_area(bytes: &[u8], area: NoteArea, found: &mut Vec<NoteOccurrence>) -> Result<()> {
    let end = checked_end(area.offset, area.size, bytes.len(), "NOTE area")?;
    let mut cursor = area.offset;
    while cursor < end {
        if end - cursor < 12 {
            bail!("truncated ELF NOTE header");
        }
        let namesz = read_u32(bytes, cursor)? as usize;
        let descsz = read_u32(bytes, cursor + 4)? as usize;
        let note_type = read_u32(bytes, cursor + 8)?;
        let name_offset = cursor + 12;
        let name_end = checked_end(name_offset, namesz, end, "NOTE name")?;
        let descriptor_offset = align4(name_end)?;
        if descriptor_offset > end {
            bail!("ELF NOTE name padding exceeds NOTE area");
        }
        let descriptor_end = checked_end(descriptor_offset, descsz, end, "NOTE descriptor")?;
        let next = align4(descriptor_end)?;
        if next > end {
            bail!("ELF NOTE padding exceeds NOTE area");
        }
        if note_type == NOTE_TYPE && &bytes[name_offset..name_end] == NOTE_NAME {
            if descsz != DESCRIPTOR_LEN {
                bail!("invalid mochiOS signature descriptor length");
            }
            if bytes[name_end..descriptor_offset]
                .iter()
                .any(|byte| *byte != 0)
                || bytes[descriptor_end..next].iter().any(|byte| *byte != 0)
            {
                bail!("non-zero mochiOS signature NOTE padding");
            }
            found.push(NoteOccurrence {
                descriptor_offset,
                area,
            });
        }
        cursor = next;
    }
    Ok(())
}

fn validate_signature_location(elf: &Elf<'_>, occurrence: NoteOccurrence) -> Result<()> {
    let descriptor = (
        occurrence.descriptor_offset,
        checked_end(
            occurrence.descriptor_offset,
            DESCRIPTOR_LEN,
            elf.bytes.len(),
            "signature descriptor",
        )?,
    );
    if overlaps(descriptor, (0, ELF64_HEADER_LEN)) {
        bail!("ELF signature descriptor overlaps the ELF header");
    }
    if elf
        .program_header_table
        .is_some_and(|table| overlaps(descriptor, table))
    {
        bail!("ELF signature descriptor overlaps the program header table");
    }
    if elf
        .section_header_table
        .is_some_and(|table| overlaps(descriptor, table))
    {
        bail!("ELF signature descriptor overlaps the section header table");
    }
    if elf
        .load_segments
        .iter()
        .copied()
        .any(|segment| overlaps(descriptor, segment))
    {
        bail!("ELF signature descriptor overlaps a PT_LOAD segment");
    }
    for (index, section) in elf.sections.iter().enumerate() {
        if section.section_type == 8 || section.size == 0 {
            continue;
        }
        let range = (section.offset, section.offset + section.size);
        let containing_signature_section = matches!(
            occurrence.area.origin,
            NoteOrigin::Section(origin) if origin == index
        );
        if !containing_signature_section && overlaps(descriptor, range) {
            bail!("ELF signature descriptor overlaps another section");
        }
    }
    Ok(())
}

fn overlaps(left: (usize, usize), right: (usize, usize)) -> bool {
    left.0 < right.1 && right.0 < left.1
}

fn parse_record(bytes: &[u8], offset: usize) -> Result<Record> {
    let descriptor =
        &bytes[offset..checked_end(offset, DESCRIPTOR_LEN, bytes.len(), "signature descriptor")?];
    if &descriptor[..8] != DESCRIPTOR_MAGIC {
        bail!("invalid ELF signature descriptor magic");
    }
    let version = read_u16(descriptor, 8)?;
    if version != VERSION {
        bail!("unsupported ELF signature version {version}");
    }
    let kind = match descriptor[10] {
        KIND_ADHOC => SignatureKind::AdHoc,
        KIND_ED25519 => SignatureKind::Ed25519,
        _ => bail!("unknown ELF signature kind"),
    };
    if descriptor[13] != 0 {
        bail!("non-zero ELF signature reserved field");
    }
    let flags = read_u32(descriptor, 14)?;
    let mut key_id = [0u8; 32];
    key_id.copy_from_slice(&descriptor[KEY_ID_OFFSET..KEY_ID_OFFSET + 32]);
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&descriptor[DIGEST_OFFSET..DIGEST_OFFSET + 32]);
    let mut signature = [0u8; 64];
    signature.copy_from_slice(&descriptor[SIGNATURE_OFFSET..SIGNATURE_OFFSET + 64]);
    Ok(Record {
        kind,
        digest_algorithm: descriptor[11],
        signature_algorithm: descriptor[12],
        flags,
        key_id,
        digest,
        signature,
        digest_file_offset: offset + DIGEST_OFFSET,
        signature_file_offset: offset + SIGNATURE_OFFSET,
    })
}

fn parse_elf(bytes: &[u8]) -> Result<Elf<'_>> {
    if bytes.len() > MAX_ELF_LEN {
        bail!("ELF exceeds the v1 size limit");
    }
    if bytes.len() < ELF64_HEADER_LEN || &bytes[..4] != ELF_MAGIC {
        bail!("invalid ELF header");
    }
    if bytes[4] != 2 || bytes[5] != 1 || bytes[6] != 1 {
        bail!("ELF Signature v1 requires ELF64 little-endian version 1");
    }
    if bytes[7] != 0 || bytes[8] != 0 || bytes[9..16].iter().any(|byte| *byte != 0) {
        bail!("unsupported ELF ABI or non-zero identification padding");
    }
    if !matches!(read_u16(bytes, 16)?, ET_EXEC | ET_DYN) {
        bail!("ELF Signature v1 supports only ET_EXEC and ET_DYN");
    }
    if read_u32(bytes, 20)? != 1 || read_u16(bytes, 52)? as usize != ELF64_HEADER_LEN {
        bail!("invalid ELF version or header size");
    }
    let phoff = usize_from_u64(read_u64(bytes, 32)?, "program header offset")?;
    let phentsize = read_u16(bytes, 54)? as usize;
    let phnum = read_u16(bytes, 56)? as usize;
    let mut program_notes = Vec::new();
    let mut load_segments = Vec::new();
    let program_header_table;
    if phnum != 0 {
        if phoff == 0 {
            bail!("ELF has program headers with a zero table offset");
        }
        if phnum == 0xffff {
            bail!("extended program header numbering is unsupported");
        }
        if phentsize != ELF64_PROGRAM_HEADER_LEN {
            bail!("invalid ELF64 program header size");
        }
        let end = checked_table(phoff, phentsize, phnum, bytes.len(), "program header table")?;
        program_header_table = Some((phoff, end));
        for index in 0..phnum {
            let raw = phoff + index * phentsize;
            let kind = read_u32(bytes, raw)?;
            let offset = usize_from_u64(read_u64(bytes, raw + 8)?, "segment offset")?;
            let size = usize_from_u64(read_u64(bytes, raw + 32)?, "segment file size")?;
            let segment_end = checked_end(offset, size, bytes.len(), "segment")?;
            let alignment = read_u64(bytes, raw + 48)?;
            if alignment > 1 && !alignment.is_power_of_two() {
                bail!("invalid program segment alignment");
            }
            if kind == PT_NOTE {
                program_notes.push(NoteArea {
                    offset,
                    size,
                    origin: NoteOrigin::ProgramHeader(index),
                });
            }
            if kind == PT_LOAD && read_u64(bytes, raw + 32)? > read_u64(bytes, raw + 40)? {
                bail!("ELF PT_LOAD file size exceeds memory size");
            }
            if kind == PT_LOAD {
                let virtual_address = read_u64(bytes, raw + 16)?;
                if alignment > 1 && virtual_address % alignment != (offset as u64) % alignment {
                    bail!("ELF PT_LOAD offset and virtual address violate alignment");
                }
                if size != 0 {
                    load_segments.push((offset, segment_end));
                }
            }
        }
    } else {
        if phoff != 0 {
            bail!("ELF has a program header offset but no entries");
        }
        program_header_table = None;
    }

    let shoff = usize_from_u64(read_u64(bytes, 40)?, "section header offset")?;
    let shentsize = read_u16(bytes, 58)? as usize;
    let shnum = read_u16(bytes, 60)? as usize;
    let shstr_index = read_u16(bytes, 62)? as usize;
    if shnum == 0 {
        bail!("ELF Signature v1 requires a Section Header Table; a future format revision may permit PT_NOTE-only discovery");
    }
    if shoff == 0 {
        bail!("ELF has sections with a zero table offset");
    }
    if shentsize != ELF64_SECTION_HEADER_LEN {
        bail!("invalid ELF64 section header size");
    }
    let section_table_end =
        checked_table(shoff, shentsize, shnum, bytes.len(), "section header table")?;
    if shstr_index == 0xffff || shstr_index >= shnum {
        bail!("invalid or extended section string table index");
    }
    let mut sections = Vec::with_capacity(shnum);
    for index in 0..shnum {
        let raw = shoff + index * shentsize;
        let offset = usize_from_u64(read_u64(bytes, raw + 24)?, "section offset")?;
        let size = usize_from_u64(read_u64(bytes, raw + 32)?, "section size")?;
        let section_type = read_u32(bytes, raw + 4)?;
        if section_type != 8 {
            checked_end(offset, size, bytes.len(), "section")?;
        } // SHT_NOBITS has no file bytes.
        sections.push(Section {
            name: read_u32(bytes, raw)?,
            section_type,
            offset,
            size,
            align: read_u64(bytes, raw + 48)?,
            entry_size: read_u64(bytes, raw + 56)?,
            raw_offset: raw,
        });
    }
    let null_section = sections[0];
    if null_section.name != 0
        || null_section.section_type != 0
        || null_section.offset != 0
        || null_section.size != 0
    {
        bail!("invalid ELF null section header");
    }
    let shstr_section = sections[shstr_index];
    if shstr_section.section_type != SHT_STRTAB {
        bail!("section name table is not SHT_STRTAB");
    }
    let shstr = &bytes[shstr_section.offset..shstr_section.offset + shstr_section.size];
    if shstr.first() != Some(&0) {
        bail!("invalid section name string table");
    }
    let elf = Elf {
        bytes,
        sections,
        shstr_index,
        shstr,
        program_notes,
        load_segments,
        program_header_table,
        section_header_table: Some((shoff, section_table_end)),
    };
    for section in &elf.sections {
        let _ = section_name(&elf, *section)?;
        if section.align != 0 && !section.align.is_power_of_two() {
            bail!("invalid section alignment");
        }
        if section.section_type != 8
            && section.size != 0
            && section.align > 1
            && (section.offset as u64) % section.align != 0
        {
            bail!("section offset violates its alignment");
        }
        if section.entry_size != 0 && section.size as u64 % section.entry_size != 0 {
            bail!("invalid section entry size");
        }
    }
    Ok(elf)
}

fn section_name<'a>(elf: &'a Elf<'_>, section: Section) -> Result<&'a [u8]> {
    let start = section.name as usize;
    if start >= elf.shstr.len() {
        bail!("section name is outside string table");
    }
    let tail = &elf.shstr[start..];
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .context("unterminated section name")?;
    Ok(&tail[..end])
}

fn checked_table(
    offset: usize,
    entry_size: usize,
    count: usize,
    limit: usize,
    name: &str,
) -> Result<usize> {
    let size = entry_size
        .checked_mul(count)
        .with_context(|| format!("{name} size overflow"))?;
    checked_end(offset, size, limit, name)
}
fn checked_end(offset: usize, size: usize, limit: usize, name: &str) -> Result<usize> {
    let end = offset
        .checked_add(size)
        .with_context(|| format!("{name} range overflow"))?;
    if end > limit {
        bail!("{name} exceeds ELF file");
    }
    Ok(end)
}
fn usize_from_u64(value: u64, name: &str) -> Result<usize> {
    usize::try_from(value).with_context(|| format!("{name} does not fit usize"))
}
fn align4(value: usize) -> Result<usize> {
    value
        .checked_add(3)
        .map(|v| v & !3)
        .context("alignment overflow")
}
fn pad_to(output: &mut Vec<u8>, alignment: usize) -> Result<()> {
    let next = output
        .len()
        .checked_add(alignment - 1)
        .context("alignment overflow")?
        & !(alignment - 1);
    output.resize(next, 0);
    Ok(())
}
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let end = offset
        .checked_add(2)
        .context("ELF integer offset overflow")?;
    let slice = bytes.get(offset..end).context("truncated ELF integer")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .context("ELF integer offset overflow")?;
    let slice = bytes.get(offset..end).context("truncated ELF integer")?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}
fn read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let end = offset
        .checked_add(8)
        .context("ELF integer offset overflow")?;
    let slice = bytes.get(offset..end).context("truncated ELF integer")?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}
fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Resolver(VerifyingKey);
    impl PublicKeyResolver for Resolver {
        fn resolve(&self, key_id: &[u8; 32]) -> Result<Option<VerifyingKey>> {
            let own: [u8; 32] = Sha256::digest(self.0.to_bytes()).into();
            Ok((own == *key_id).then_some(self.0))
        }
    }

    fn fixture() -> Vec<u8> {
        let shstr = b"\0.shstrtab\0.text\0";
        let mut elf = vec![0u8; 0x100];
        elf[..4].copy_from_slice(ELF_MAGIC);
        elf[4] = 2;
        elf[5] = 1;
        elf[6] = 1;
        put_u16(&mut elf, 16, ET_EXEC);
        put_u16(&mut elf, 18, 62);
        put_u32(&mut elf, 20, 1);
        put_u64(&mut elf, 24, 0x80);
        put_u16(&mut elf, 52, 64);
        put_u16(&mut elf, 58, 64);
        elf[0x80..0x84].copy_from_slice(&[0x90, 0x90, 0xc3, 0]);
        elf[0x90..0x90 + shstr.len()].copy_from_slice(shstr);
        let table = 0x100;
        elf.resize(table + 3 * 64, 0);
        put_u64(&mut elf, 40, table as u64);
        put_u16(&mut elf, 60, 3);
        put_u16(&mut elf, 62, 1);
        put_u32(&mut elf, table + 64, 1);
        put_u32(&mut elf, table + 68, SHT_STRTAB);
        put_u64(&mut elf, table + 64 + 24, 0x90);
        put_u64(&mut elf, table + 64 + 32, shstr.len() as u64);
        put_u64(&mut elf, table + 64 + 48, 1);
        put_u32(&mut elf, table + 128, 11);
        put_u32(&mut elf, table + 132, 1);
        put_u64(&mut elf, table + 128 + 24, 0x80);
        put_u64(&mut elf, table + 128 + 32, 4);
        put_u64(&mut elf, table + 128 + 48, 16);
        elf
    }

    #[test]
    fn adhoc_round_trip_and_tamper_detection() {
        let signed = sign_adhoc(&fixture()).unwrap();
        let decoded = decode(&signed).unwrap();
        assert_eq!(decoded.kind, SignatureKind::AdHoc);
        assert_eq!(decoded.digest_algorithm, DIGEST_SHA256);
        assert_eq!(decoded.signature_algorithm, SIGNATURE_NONE);
        assert_eq!(decoded.flags, 0);
        assert_eq!(decoded.key_id, [0; 32]);
        assert_eq!(decoded.signature, [0; 64]);
        assert_eq!(verify(&signed, None).unwrap().kind, SignatureKind::AdHoc);
        let mut tampered = signed.clone();
        tampered[0x80] ^= 1;
        assert!(verify(&tampered, None)
            .unwrap_err()
            .to_string()
            .contains("digest mismatch"));
        assert!(sign_adhoc(&signed)
            .unwrap_err()
            .to_string()
            .contains("already contains"));
    }

    #[test]
    fn keyed_round_trip_is_fail_closed_and_binds_key_id() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let signed = sign_ed25519(&fixture(), &key).unwrap();
        assert!(verify(&signed, None)
            .unwrap_err()
            .to_string()
            .contains("public key is required"));
        let verified = verify(&signed, Some(&Resolver(key.verifying_key()))).unwrap();
        assert_eq!(verified.kind, SignatureKind::Ed25519);
        assert_eq!(
            verified.key_id.unwrap(),
            Sha256::digest(key.verifying_key().to_bytes()).as_slice()
        );
        let wrong = SigningKey::from_bytes(&[8u8; 32]).verifying_key();
        assert!(verify(&signed, Some(&Resolver(wrong)))
            .unwrap_err()
            .to_string()
            .contains("unresolved"));
    }

    #[test]
    fn malformed_unknown_version_and_signature_tampering_are_rejected() {
        let key = SigningKey::from_bytes(&[9u8; 32]);
        let signed = sign_ed25519(&fixture(), &key).unwrap();
        let record = locate_record(&signed).unwrap().unwrap();
        let descriptor = record.digest_file_offset - DIGEST_OFFSET;
        let mut version = signed.clone();
        version[descriptor + 8] = 2;
        assert!(verify(&version, Some(&Resolver(key.verifying_key())))
            .unwrap_err()
            .to_string()
            .contains("unsupported ELF signature version"));
        let mut signature = signed.clone();
        signature[record.signature_file_offset] ^= 1;
        assert!(verify(&signature, Some(&Resolver(key.verifying_key()))).is_err());
        let mut malformed = signed;
        put_u64(&mut malformed, 40, u64::MAX);
        assert!(verify(&malformed, None).is_err());
    }

    #[test]
    fn duplicate_note_is_rejected_even_when_two_headers_reference_one_note() {
        let signed = sign_adhoc(&fixture()).unwrap();
        let parsed = parse_elf(&signed).unwrap();
        let signature_section = parsed
            .sections
            .iter()
            .find(|section| section.section_type == SHT_NOTE)
            .unwrap();
        let old_table = read_u64(&signed, 40).unwrap() as usize;
        let old_count = read_u16(&signed, 60).unwrap() as usize;
        let mut duplicate = signed.clone();
        pad_to(&mut duplicate, 8).unwrap();
        let new_table = duplicate.len();
        duplicate.extend_from_slice(&signed[old_table..old_table + old_count * 64]);
        duplicate.extend_from_slice(
            &signed[signature_section.raw_offset..signature_section.raw_offset + 64],
        );
        put_u64(&mut duplicate, 40, new_table as u64);
        put_u16(&mut duplicate, 60, (old_count + 1) as u16);
        assert!(verify(&duplicate, None)
            .unwrap_err()
            .to_string()
            .contains("duplicate"));
    }

    #[test]
    fn two_physical_signature_notes_are_rejected() {
        let signed = sign_adhoc(&fixture()).unwrap();
        let parsed = parse_elf(&signed).unwrap();
        let signature_section = parsed
            .sections
            .iter()
            .find(|section| section.section_type == SHT_NOTE)
            .unwrap();
        let old_table = read_u64(&signed, 40).unwrap() as usize;
        let old_count = read_u16(&signed, 60).unwrap() as usize;
        let note = signed
            [signature_section.offset..signature_section.offset + signature_section.size]
            .to_vec();

        let mut duplicate = signed.clone();
        pad_to(&mut duplicate, 4).unwrap();
        let second_note = duplicate.len();
        duplicate.extend_from_slice(&note);
        pad_to(&mut duplicate, 8).unwrap();
        let new_table = duplicate.len();
        duplicate.extend_from_slice(&signed[old_table..old_table + old_count * 64]);
        let second_header = duplicate.len();
        duplicate.extend_from_slice(
            &signed[signature_section.raw_offset
                ..signature_section.raw_offset + ELF64_SECTION_HEADER_LEN],
        );
        put_u64(&mut duplicate, second_header + 24, second_note as u64);
        put_u64(&mut duplicate, 40, new_table as u64);
        put_u16(&mut duplicate, 60, (old_count + 1) as u16);

        assert!(verify(&duplicate, None)
            .unwrap_err()
            .to_string()
            .contains("duplicate"));
    }

    #[test]
    fn signing_message_layout_and_little_endian_flags_are_fixed() {
        let key_id = [0x55; 32];
        let digest = [0xaa; 32];
        let message = signing_message(
            KIND_ED25519,
            DIGEST_SHA256,
            SIGNATURE_ED25519,
            0x0102_0304,
            &key_id,
            &digest,
        );
        assert_eq!(message.len(), 96);
        assert_eq!(&message[..25], b"mochios-elf-signature-v1\0");
        assert_eq!(
            &message[25..28],
            &[KIND_ED25519, DIGEST_SHA256, SIGNATURE_ED25519]
        );
        assert_eq!(&message[28..32], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&message[32..64], &key_id);
        assert_eq!(&message[64..96], &digest);
    }

    #[test]
    fn unknown_algorithm_flags_and_invalid_note_length_are_rejected() {
        let signed = sign_adhoc(&fixture()).unwrap();
        let record = locate_record(&signed).unwrap().unwrap();
        let descriptor = record.digest_file_offset - DIGEST_OFFSET;

        let mut algorithm = signed.clone();
        algorithm[descriptor + 11] = 0xff;
        assert!(decode(&algorithm)
            .unwrap_err()
            .to_string()
            .contains("unsupported ELF digest algorithm"));

        let mut flags = signed.clone();
        flags[descriptor + 14] = 1;
        assert!(decode(&flags)
            .unwrap_err()
            .to_string()
            .contains("unsupported ELF signature flags"));

        let note_header = descriptor - NOTE_NAME.len() - 12;
        let mut length = signed;
        put_u32(&mut length, note_header + 4, (DESCRIPTOR_LEN - 1) as u32);
        assert!(decode(&length)
            .unwrap_err()
            .to_string()
            .contains("descriptor length"));
    }

    #[test]
    fn sectionless_signing_constraint_is_explicit() {
        let mut elf = fixture();
        put_u64(&mut elf, 40, 0);
        put_u16(&mut elf, 58, 0);
        put_u16(&mut elf, 60, 0);
        put_u16(&mut elf, 62, 0);
        elf.truncate(0x100);
        assert!(sign_adhoc(&elf)
            .unwrap_err()
            .to_string()
            .contains("requires a Section Header Table"));
    }
}
