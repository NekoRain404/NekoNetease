//! The updater couplings that span trees, where neither half can see the other break.
//!
//! Each one fails silently in CI and loudly in an install: a renamed trusted-comment token makes
//! every client reject the release as `VersionMismatch`, a platform key the manifest stops naming
//! leaves that format's users on "no update available" indefinitely (v0.8.0's deb slot, which is
//! why the script carries a comment about it), a schema default that drifts from the client's
//! constant makes every client refuse the manifest, and a swapped public key is an unintended
//! rotation. The version the manifest carries and the artifacts it scans are the same shape: a
//! `v`-prefixed version parses as no update on every client, and a package left outside `dist`
//! reaches no manifest at all. None of them has an anchor file, because the argument is the
//! agreement itself.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use nekonetease_app::services::updater::manifest::SUPPORTED_MANIFEST_SCHEMA;
use nekonetease_testkit::{REPO_ROOT, strip_line_comments};

/// One signing preflight, one artifact-signing call per packaging OS (Linux and Windows) and one
/// release-manifest signing call. An equality rather than a floor: any new caller must pass the
/// same fields, and a removed caller must update this contract.
const SIGNING_INVOCATIONS: usize = 4;

/// Ten from `format_key`, twelve from the script. Equalities so that adding a platform has to be
/// done on both sides at once rather than on whichever one the change started in.
const CLIENT_KEYS: usize = 10;
const MANIFEST_KEYS: usize = 12;

fn read(path: &Path) -> String {
    let src = fs::read_to_string(path).unwrap_or_default();
    assert!(!src.is_empty(), "unreadable or empty: {}", path.display());
    src
}

/// Every workflow under `.github/workflows/`, so a new one signing without the fields is caught
/// by being walked rather than by being remembered.
fn workflows() -> Vec<(String, String)> {
    let dir = Path::new(REPO_ROOT).join(".github/workflows");
    let listing = fs::read_dir(&dir);
    assert!(listing.is_ok(), "`{}` would not list", dir.display());

    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    for entry in listing.into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("yml") {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        match fs::read_to_string(&path) {
            Ok(text) => found.push((name, text)),
            Err(_) => unreadable.push(name),
        }
    }

    // Counted rather than skipped: a workflow that fails to read is one this walk would otherwise
    // clear without looking at.
    assert!(unreadable.is_empty(), "unreadable workflows: {unreadable:?}");
    assert!(!found.is_empty(), "no workflows found under `{}`", dir.display());
    found.sort();
    found
}

fn trusted_comments(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|line| line.trim().strip_prefix("trusted-comment:"))
        .map(|value| value.trim().to_owned())
        .collect()
}

/// Every double-quoted literal shaped like a `latest.json` platform key. Both halves of the
/// comparison spell their keys as literals in a table, so reading them back is the only way to
/// ask whether the two tables still agree.
fn platform_keys(src: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let mut rest = src;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        let literal = &after[..close];
        if literal.starts_with("linux-") || literal.starts_with("windows-") {
            keys.insert(literal.to_owned());
        }
        rest = &after[close + 1..];
    }
    keys
}

/// `minisign.rs` parses `version=` out of every signature and `manifest=` out of the manifest's,
/// and refuses the install when either is missing. The release workflow produces both tokens and
/// nothing else asks whether the two vocabularies still match.
#[test]
fn every_signature_carries_the_trusted_comment_fields_the_client_parses() {
    let mut comments = Vec::new();
    let mut invocations = 0;
    for (name, src) in workflows() {
        invocations += src.matches("uses: ./.github/actions/minisign-sign").count();
        comments.extend(trusted_comments(&src).into_iter().map(|value| (name.clone(), value)));
    }

    assert_eq!(
        invocations, SIGNING_INVOCATIONS,
        "the signing action is used {invocations} times; a new caller owes the same fields"
    );
    assert_eq!(
        comments.len(),
        SIGNING_INVOCATIONS,
        "every minisign-sign call must pass a trusted comment, got {comments:?}"
    );

    for (name, comment) in &comments {
        assert!(
            comment.contains("version="),
            "{name}: the client asserts version= against the manifest it is installing: {comment}"
        );
    }

    let manifests: Vec<&String> =
        comments.iter().filter(|(_, c)| c.contains("manifest=true")).map(|(n, _)| n).collect();
    let preflights: Vec<&String> =
        comments.iter().filter(|(_, c)| c.contains("preflight=true")).map(|(n, _)| n).collect();
    let artifacts: Vec<&(String, String)> =
        comments.iter().filter(|(_, c)| c.contains("file={file}")).collect();

    // `manifest=true` is the domain separation that stops an artifact signature being swapped into
    // the manifest's slot; the release workflow's manifest signature owes it.
    assert_eq!(manifests.len(), 1, "expected one manifest signature, got {manifests:?}");
    assert_eq!(preflights.len(), 1, "expected one signing preflight, got {preflights:?}");
    assert_eq!(
        artifacts.len(),
        2,
        "expected the Linux and Windows artifact signatures, got {artifacts:?}"
    );
    for (name, artifact) in &artifacts {
        assert!(
            artifact.contains("target="),
            "{name}: the artifact comment names the target it was built for: {artifact}"
        );
    }
}

/// The `-t` flag itself lives in the composite action all callers delegate to, so the
/// fields above reach a signature only while this step still passes them. `-SH` is the other half:
/// a signature that isn't prehashed can't be verified as the artifact streams, and the client
/// rejects it outright.
#[test]
fn the_signing_step_still_prehashes_and_writes_the_comment() {
    let path = Path::new(REPO_ROOT).join(".github/actions/minisign-sign/action.yml");
    let src = read(&path);

    assert!(src.contains("minisign -SHm"), "signing must stay prehashed");
    assert!(src.contains("-t \"${COMMENT"), "the trusted comment must still be signed");
    assert!(
        src.contains("${COMMENT//\\{file\\}/"),
        "the per-artifact comment depends on {{file}} being substituted"
    );
}

/// `format_key` produces the keys a client asks for; `build-latest-json.py` decides which ones the
/// manifest carries. A key on the client's side alone reaches the user as a failed update naming
/// a platform asset that was never published.
#[test]
fn every_platform_key_the_client_asks_for_is_one_the_manifest_can_carry() {
    let client = platform_keys(&strip_line_comments(&read(
        &Path::new(REPO_ROOT)
            .join("crates/nekonetease-platform/src/services/platform/install_kind/target.rs"),
    )));
    let manifest = platform_keys(&read(&Path::new(REPO_ROOT).join("scripts/build-latest-json.py")));

    assert_eq!(client.len(), CLIENT_KEYS, "client keys drifted: {client:?}");
    assert_eq!(manifest.len(), MANIFEST_KEYS, "manifest keys drifted: {manifest:?}");

    let unpublishable: Vec<&String> = client.difference(&manifest).collect();
    assert!(
        unpublishable.is_empty(),
        "{unpublishable:?} can be asked for but never appears in a manifest — the filename \
         patterns are downstream of the packaging steps and must move with them"
    );
}

/// `check.rs` refuses any manifest declaring a schema above the constant, so the value CI writes
/// and the value the client understands are one protocol. The constant used to be pinned against
/// itself with a comment telling the reader to keep the script in step by hand.
#[test]
fn the_manifest_schema_the_client_supports_is_the_one_ci_writes() {
    let src = read(&Path::new(REPO_ROOT).join("scripts/build-latest-json.py"));
    let flag = src.find("\"--manifest-schema-version\"");
    assert!(flag.is_some(), "the script no longer declares --manifest-schema-version");

    let written = flag
        .and_then(|at| src[at..].find("default=").map(|d| at + d + "default=".len()))
        .map(|at| src[at..].chars().take_while(char::is_ascii_digit).collect::<String>())
        .and_then(|digits| digits.parse::<u32>().ok());

    assert_eq!(
        written,
        Some(SUPPORTED_MANIFEST_SCHEMA),
        "the script's default schema and the client's supported schema must move together"
    );
}

/// Rotation is documented as one-way and disruptive; an accidental rotation is the same event
/// without the intent, and a fixture key committed over this file would still parse. Only the key
/// material is pinned — the untrusted comment above it is free text minisign never verifies.
#[test]
fn the_embedded_pubkey_is_still_the_release_key() {
    let path = Path::new(REPO_ROOT).join("assets/updater-pubkey.b64");
    let src = read(&path);

    let key = src.lines().map(str::trim).filter(|line| !line.is_empty()).nth(1);
    assert_eq!(
        key,
        Some("RWTwuz63tN8KD4YHtaf5moWcPnLxfqhqC+crApAYcUECzjkK5ii0tklz"),
        "the updater's public key changed; every install already in the field verifies against \
         the old one, so this is only correct as a deliberate rotation"
    );
}

/// The Windows client asks for `windows-x86_64-msi`, and `check.rs` reports the update as
/// available before any install can discover the asset is missing. An update that says
/// "available" and then cannot install is worse than no update, so both formats the client
/// classifies are built, uploaded and signed by name.
#[test]
fn the_windows_target_has_a_built_and_signed_asset() {
    let src = read(&Path::new(REPO_ROOT).join(".github/workflows/release.yml"));

    let publish = src.split_once("\n  publish:\n").map(|(_, after)| after).unwrap_or_default();

    for name in [
        "NekoNetease-${{ github.ref_name }}-x86_64-windows.zip",
        "NekoNetease-${{ github.ref_name }}-x86_64.msi",
    ] {
        let artifact = format!("dist/{name}");
        assert!(src.contains(&artifact), "the Windows leg no longer produces {artifact}");
        assert!(
            publish.contains(&artifact),
            "publish does not sign {artifact}; the manifest skips an unsigned artifact"
        );
    }
    assert!(
        src.contains("throw \"cargo wix produced no MSI"),
        "the MSI step must fail when cargo-wix writes nothing, or the upload silently ships \
         the ZIP alone and the Windows client has no asset"
    );
}

/// `is_upgrade` parses the manifest's `version` with `semver::Version`, which rejects a leading
/// `v`, and the Linux smoke test looks for that same string in `--version` output. Both must be
/// the workspace version `prepare-release.py` rewrites from the tag, never the tag itself.
#[test]
fn the_manifest_version_is_the_prepared_workspace_version() {
    let src = read(&Path::new(REPO_ROOT).join(".github/workflows/release.yml"));
    let script = read(&Path::new(REPO_ROOT).join("scripts/build-latest-json.py"));

    assert!(
        src.contains("run: python3 scripts/prepare-release.py"),
        "the build and publish jobs must take their version from prepare-release.py"
    );
    assert_eq!(
        src.matches("GITHUB_EVENT_NAME: release").count(),
        2,
        "both the build and publish checkouts need the tag prepared into the workspace version"
    );
    assert!(
        src.contains("run: cargo update --workspace"),
        "prepare-release rewrites Cargo.toml, and `cargo build --locked` rejects the stale lock"
    );

    let publish = src.split_once("\n  publish:\n").map(|(_, after)| after).unwrap_or_default();
    assert!(
        publish.contains("version: ${{ steps.release.outputs.version }}"),
        "the manifest must be given the workspace version"
    );
    assert!(
        publish.contains("tag: ${{ github.ref_name }}"),
        "the manifest's tag field stays the v-prefixed tag"
    );
    assert!(
        !publish.contains("version: ${{ github.ref_name }}"),
        "the tag is not the manifest version: `is_upgrade` rejects its prefix"
    );
    assert!(
        script.contains("parser.error(\"--version must be the bare"),
        "the generator must refuse a prefixed version rather than emit an unusable manifest"
    );
}

/// Checksums ship beside the packages they cover. The manifest build refuses a `dist` file it
/// cannot classify, so the file must be written after that scan — and before the manifest
/// signature, whose `.minisig` is not a package's sibling.
#[test]
fn the_release_ships_checksums_over_the_signed_packages() {
    let src = read(&Path::new(REPO_ROOT).join(".github/workflows/release.yml"));
    let publish = src.split_once("\n  publish:\n").map(|(_, after)| after).unwrap_or_default();

    let mut positions = Vec::new();
    for step in [
        "- name: Build the updater manifest",
        "- name: Write SHA256SUMS.txt",
        "- name: Sign the manifest",
    ] {
        let at = publish.find(step);
        assert!(at.is_some(), "publish no longer has a {step:?} step");
        positions.push(at.unwrap_or_default());
    }
    assert!(
        positions[0] < positions[1] && positions[1] < positions[2],
        "SHA256SUMS.txt must be written after the manifest scans `dist` and before the \
         manifest's signature is added: {positions:?}"
    );

    assert!(
        publish.contains("working-directory: dist"),
        "the checksums must be built from the directory the manifest scans"
    );
    assert!(
        publish.contains("LC_ALL=C sort"),
        "the checksum lines must be sorted deterministically"
    );
    assert!(
        publish.contains("> SHA256SUMS.txt"),
        "publish must write the checksum file into `dist` for the upload step"
    );
    for subject in
        ["'*.tar.gz'", "'*.AppImage'", "'*.rpm'", "'*.deb'", "'*.msi'", "'*.zip'", "'*.minisig'"]
    {
        assert!(
            publish.contains(subject),
            "the checksum set no longer covers {subject}; every package and its signature ships"
        );
    }
}

/// One attestation step in the shared matrix attests both slots; a step written per OS is how
/// one slot silently ships packages with no provenance. The default token carries neither
/// permission the statement needs, so they have to be granted at the job that runs it.
#[test]
fn every_packaging_slot_attests_its_packages() {
    let src = read(&Path::new(REPO_ROOT).join(".github/workflows/release.yml"));
    let build = src.split_once("\n  publish:\n").map(|(before, _)| before).unwrap_or_default();

    assert_eq!(
        build.matches("uses: actions/attest-build-provenance@").count(),
        1,
        "a single step in the shared matrix attests every slot"
    );
    let line = build
        .lines()
        .find(|line| line.contains("uses: actions/attest-build-provenance@"))
        .unwrap_or_default();
    let sha =
        line.split('@').nth(1).and_then(|rest| rest.split_whitespace().next()).unwrap_or_default();
    assert_eq!(
        sha.len(),
        40,
        "the attestation action must be pinned to a full commit SHA, not a moving tag: {line}"
    );
    assert!(
        sha.chars().all(|c| c.is_ascii_hexdigit()),
        "the attestation action's pin is not a commit SHA: {line}"
    );
    assert!(line.contains("# v4.2.2"), "the pin must carry its version comment: {line}");

    assert!(build.contains("id-token: write"), "the attestation needs its OIDC token");
    assert!(
        build.contains("attestations: write"),
        "the attestation needs permission to write the record"
    );
    for package in
        ["dist/*.tar.gz", "dist/*.AppImage", "dist/*.rpm", "dist/*.deb", "dist/*.msi", "dist/*.zip"]
    {
        assert!(
            build.contains(package),
            "the attestation no longer covers {package}; the slot attests what it packaged"
        );
    }
}

/// The manifest build scans one flat directory after the matrix, while the Linux package
/// scripts write into the repo root and `~/rpmbuild`. Without an explicit staging step the
/// Linux half of every manifest is missing, and cargo-deb's `_amd64.deb` spelling matches no
/// pattern even when it does arrive.
#[test]
fn every_linux_package_lands_in_the_directory_the_manifest_reads() {
    let src = read(&Path::new(REPO_ROOT).join(".github/workflows/release.yml"));
    let build = src.split_once("\n  publish:\n").map(|(before, _)| before).unwrap_or_default();

    for staged in [
        "mkdir -p dist",
        "dist/nekonetease-v${{ steps.release.outputs.version }}-x86_64-linux.tar.gz",
        "dist/NekoNetease-${{ steps.release.outputs.version }}-x86_64.AppImage",
        "cp \"$HOME\"/rpmbuild/RPMS/x86_64/nekonetease-*.rpm dist/",
        "dist/nekonetease-${{ steps.release.outputs.version }}-x86_64.deb",
        "find dist -maxdepth 1 -type f -print",
    ] {
        assert!(build.contains(staged), "the Linux leg no longer stages {staged}");
    }
}
