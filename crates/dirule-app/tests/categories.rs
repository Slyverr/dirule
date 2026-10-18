use dirule_app::{
    CURRENT_CATEGORY_CATALOG_VERSION, CategoryCatalog, CategoryCatalogError,
    sort_all_categories_preview, sort_extension_preview, sort_selectors_preview,
};
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP_TARGET: AtomicU64 = AtomicU64::new(0);

struct TempTarget(PathBuf);

impl TempTarget {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEMP_TARGET.fetch_add(1, Ordering::Relaxed);
        let target = std::env::temp_dir().join(format!(
            "dirule-categories-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&target).expect("create target");

        Self(target)
    }
}

impl Deref for TempTarget {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TempTarget {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn portable_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn built_in_categories_cover_common_file_groups() {
    let categories = CategoryCatalog::built_in();

    assert_eq!(
        categories.get("images").unwrap().destination,
        "media/images"
    );
    assert_eq!(
        categories.get("videos").unwrap().destination,
        "media/videos"
    );
    assert_eq!(categories.get("audio").unwrap().destination, "media/audio");
    assert_eq!(
        categories.get("documents").unwrap().destination,
        "documents"
    );
    assert!(
        categories
            .get("images")
            .unwrap()
            .extensions
            .contains(&"png".into())
    );
    assert!(
        categories
            .get("videos")
            .unwrap()
            .extensions
            .contains(&"mp4".into())
    );
    assert!(
        categories
            .get("documents")
            .unwrap()
            .extensions
            .contains(&"pdf".into())
    );
}

#[test]
fn user_categories_add_and_override_built_ins() {
    let source = format!(
        "version = \"{CURRENT_CATEGORY_CATALOG_VERSION}\"\n\
         [[categories]]\nname = 'images'\nextensions = ['raw', '.CR2']\ndestination = 'photos'\n\
         [[categories]]\nname = 'ebooks'\nextensions = ['epub']\ndestination = 'reading/books'\n"
    );
    let catalog =
        CategoryCatalog::parse(Path::new("categories.toml"), &source).expect("valid catalog");

    assert_eq!(catalog.get("images").unwrap().extensions, ["cr2", "raw"]);
    assert_eq!(catalog.get("images").unwrap().destination, "photos");
    assert_eq!(catalog.get("ebooks").unwrap().destination, "reading/books");
    assert!(catalog.get("videos").is_some());
}

#[test]
fn category_catalog_rejects_unsafe_destinations_and_unknown_fields() {
    for field in [
        "destination = '../outside'",
        "destination = '/absolute'",
        "extra = true",
    ] {
        let source = format!(
            "version = \"{CURRENT_CATEGORY_CATALOG_VERSION}\"\n\
             [[categories]]\nname = 'ebooks'\nextensions = ['epub']\n{field}\n"
        );
        let error = CategoryCatalog::parse(Path::new("categories.toml"), &source)
            .expect_err("invalid catalog should fail");

        assert!(error.to_string().contains("categories.toml"));
    }
}

#[test]
fn category_catalog_rejects_unknown_versions_and_duplicate_extensions() {
    let future = CategoryCatalog::parse(
        Path::new("categories.toml"),
        "version = '2.0'\n[[categories]]\nname = 'books'\nextensions = ['epub']\ndestination = 'books'\n",
    )
    .expect_err("unknown version should fail");
    assert!(future.to_string().contains("unsupported"));

    let duplicate = format!(
        "version = \"{CURRENT_CATEGORY_CATALOG_VERSION}\"\n\
         [[categories]]\nname = 'books'\nextensions = ['epub', '.EPUB']\ndestination = 'books'\n"
    );
    let error = CategoryCatalog::parse(Path::new("categories.toml"), &duplicate)
        .expect_err("duplicate extensions should fail");
    assert!(error.to_string().contains("repeated"));
}

#[test]
fn category_catalog_rejects_multiple_leading_dots_in_extensions() {
    let source = format!(
        "version = \"{CURRENT_CATEGORY_CATALOG_VERSION}\"\n\
         [[categories]]\nname = 'ebooks'\nextensions = ['..PDF']\ndestination = 'books'\n"
    );

    let error = CategoryCatalog::parse(Path::new("categories.toml"), &source)
        .expect_err("only one optional leading dot is valid");

    assert!(error.to_string().contains("invalid extension"));
}

#[test]
fn missing_user_catalog_uses_built_in_categories() {
    let target = TempTarget::new();
    let catalog = CategoryCatalog::load_optional(&target.join("missing.toml"))
        .expect("missing optional catalog should use defaults");

    assert!(catalog.get("images").is_some());
    assert!(catalog.get("documents").is_some());
}

#[test]
fn sort_resolves_multiple_extensions_and_categories_through_rules() {
    let target = TempTarget::new();
    fs::write(target.join("Portrait.JPG"), "image").expect("write image");
    fs::write(target.join("Clip.MP4"), "video").expect("write video");
    fs::write(target.join("Guide.PDF"), "document").expect("write document");
    fs::write(target.join("Notes.TXT"), "text").expect("write extension file");
    let selectors = ["images", "videos", "pdf", "txt"].map(str::to_owned);
    let plan = sort_selectors_preview(&target, &selectors, &CategoryCatalog::built_in(), false)
        .expect("preview mixed selectors");

    let destinations = plan
        .plan()
        .entries
        .iter()
        .map(|entry| {
            (
                portable_path(&entry.source),
                portable_path(&entry.destination),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(destinations["Portrait.JPG"], "media/images/Portrait.JPG");
    assert_eq!(destinations["Clip.MP4"], "media/videos/Clip.MP4");
    assert_eq!(destinations["Guide.PDF"], "pdfs/Guide.PDF");
    assert_eq!(destinations["Notes.TXT"], "txts/Notes.TXT");
    assert!(target.join("Portrait.JPG").is_file());
    assert!(!target.join("media/images/Portrait.JPG").exists());
}

#[test]
fn sort_all_categories_uses_catalog_and_leaves_unmatched_or_nested_files_alone() {
    let target = TempTarget::new();
    fs::write(target.join("Portrait.JPG"), "image").expect("write image");
    fs::write(target.join("Clip.MP4"), "video").expect("write video");
    fs::write(target.join("Voice.MP3"), "audio").expect("write audio");
    fs::write(target.join("Guide.PDF"), "document").expect("write document");
    fs::write(target.join("payload.bin"), "unknown").expect("write unmatched file");
    fs::create_dir_all(target.join("nested")).expect("create nested directory");
    fs::write(target.join("nested/Deep.JPG"), "nested image").expect("write nested image");

    let preview = sort_all_categories_preview(&target, &CategoryCatalog::built_in(), false)
        .expect("preview all built-in categories");
    let destinations = preview
        .plan()
        .entries
        .iter()
        .map(|entry| {
            (
                portable_path(&entry.source),
                portable_path(&entry.destination),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(destinations["Portrait.JPG"], "media/images/Portrait.JPG");
    assert_eq!(destinations["Clip.MP4"], "media/videos/Clip.MP4");
    assert_eq!(destinations["Voice.MP3"], "media/audio/Voice.MP3");
    assert_eq!(destinations["Guide.PDF"], "documents/Guide.PDF");
    assert_eq!(destinations["payload.bin"], "payload.bin");
    assert_eq!(
        destinations["nested/Deep.JPG"], "nested/Deep.JPG",
        "nested files stay in place without recursive sorting"
    );
    assert!(target.join("Portrait.JPG").is_file());
    assert!(!target.join("media/images/Portrait.JPG").exists());
}

#[test]
fn category_overrides_control_extensions_and_destinations() {
    let target = TempTarget::new();
    fs::write(target.join("Camera.RAW"), "raw").expect("write camera raw");
    let catalog_source = format!(
        "version = \"{CURRENT_CATEGORY_CATALOG_VERSION}\"\n\
         [[categories]]\nname = 'images'\nextensions = ['raw']\ndestination = 'photos'\n"
    );
    let catalog = CategoryCatalog::parse(Path::new("categories.toml"), &catalog_source)
        .expect("parse category override");
    let selectors = ["images".to_owned()];
    let plan = sort_selectors_preview(&target, &selectors, &catalog, false)
        .expect("preview overridden category");

    assert!(plan.plan().entries.iter().any(|entry| {
        entry.source == Path::new("Camera.RAW")
            && entry.destination == Path::new("photos/Camera.RAW")
    }));
}

#[test]
fn single_extension_preview_keeps_extension_semantics_for_category_names() {
    let target = TempTarget::new();
    fs::write(target.join("Artwork.images"), "file").expect("write category-named extension");

    let preview =
        sort_extension_preview(&target, "images", false).expect("sort the literal extension name");

    assert!(preview.plan().entries.iter().any(|entry| {
        entry.source == Path::new("Artwork.images")
            && entry.destination == Path::new("imagess/Artwork.images")
    }));
}

#[test]
fn overlapping_selected_categories_are_rejected() {
    let target = TempTarget::new();
    let source = format!(
        "version = \"{CURRENT_CATEGORY_CATALOG_VERSION}\"\n\
         [[categories]]\nname = 'photos'\nextensions = ['png']\ndestination = 'photos'\n"
    );
    let catalog = CategoryCatalog::parse(Path::new("categories.toml"), &source)
        .expect("parse custom category");
    let selectors = ["images", "photos"].map(str::to_owned);

    let error = sort_selectors_preview(&target, &selectors, &catalog, false)
        .err()
        .expect("overlapping selectors should fail");

    assert!(error.to_string().contains("overlap"));
}

#[test]
fn category_catalog_error_keeps_invalid_source_path() {
    let path = Path::new("user-config/dirule/categories.toml");
    let error =
        CategoryCatalog::parse(path, "version = 7").expect_err("invalid catalog should fail");

    assert!(matches!(error, CategoryCatalogError::Invalid { .. }));
    assert!(
        error
            .to_string()
            .contains("user-config/dirule/categories.toml")
    );
}
