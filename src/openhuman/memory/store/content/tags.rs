//! Host adapter for TinyCortex-owned markdown tag rewriting.
//!
//! Generic chunk rewrites and tag formatting live in TinyCortex. OpenHuman
//! retains only the summary adapter because it resolves product configuration,
//! content pointers, and entity-index rows.

use std::path::Path;

use crate::openhuman::config::Config;
use crate::openhuman::memory::store::chunks::store::get_summary_content_pointers;
use crate::openhuman::memory::store::content::compose::{
    rewrite_summary_tags, scan_fm_field, source_tag, split_front_matter,
};
use crate::openhuman::memory::tree::score::store::list_entity_ids_for_node;

pub use tinycortex::memory::store::content::tags::{
    entity_tag, slugify_tag_kind, slugify_tag_value, update_chunk_tags,
};

/// Rewrite a summary's tags from its authoritative entity-index rows.
///
/// This is host-owned glue: TinyCortex performs the generic markdown rewrite,
/// while OpenHuman supplies configuration and entity-index lookup.
pub fn update_summary_tags(config: &Config, summary_id: &str) -> anyhow::Result<()> {
    let Some((rel_path, expected_sha)) = get_summary_content_pointers(config, summary_id)? else {
        log::debug!(
            "[content_store::tags] update_summary_tags: no content_path for summary {summary_id} — skipping"
        );
        return Ok(());
    };

    let mut abs_path = config.memory_tree_content_root();
    for component in rel_path.split('/') {
        abs_path.push(component);
    }
    if !abs_path.exists() {
        log::debug!(
            "[content_store::tags] update_summary_tags: file missing for summary {summary_id} at {} — skipping",
            abs_path.display()
        );
        return Ok(());
    }

    let mut tags = list_entity_ids_for_node(config, summary_id)?
        .iter()
        .filter_map(|entity_id| {
            let (kind, surface) = entity_id.split_once(':')?;
            Some(entity_tag(kind, surface))
        })
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();

    let old_bytes = std::fs::read(&abs_path)
        .map_err(|error| anyhow::anyhow!("read summary {:?}: {error}", abs_path))?;
    let tags = augment_with_source_tag(&old_bytes, &tags);
    let new_bytes = rewrite_summary_tags(&old_bytes, &tags)
        .map_err(|error| anyhow::anyhow!("rewrite_summary_tags {:?}: {error}", abs_path))?;

    write_atomically(&abs_path, &new_bytes)?;

    let verify_bytes = std::fs::read(&abs_path)
        .map_err(|error| anyhow::anyhow!("re-read after tag rewrite {:?}: {error}", abs_path))?;
    let content = std::str::from_utf8(&verify_bytes)
        .map_err(|error| anyhow::anyhow!("UTF-8 after tag rewrite {:?}: {error}", abs_path))?;
    let body = split_front_matter(content)
        .ok_or_else(|| anyhow::anyhow!("no front-matter after tag rewrite {:?}", abs_path))?
        .1;
    let actual_sha = super::atomic::sha256_hex(body.as_bytes());
    if actual_sha != expected_sha {
        return Err(anyhow::anyhow!(
            "[content_store::tags] update_summary_tags body mutated after rewrite summary_id={summary_id} expected_sha={expected_sha} actual_sha={actual_sha}"
        ));
    }

    log::debug!(
        "[content_store::tags] updated summary tags summary_id={summary_id} n_tags={}",
        tags.len()
    );
    Ok(())
}

fn augment_with_source_tag(file_bytes: &[u8], tags: &[String]) -> Vec<String> {
    let Some(front_matter) = std::str::from_utf8(file_bytes)
        .ok()
        .and_then(split_front_matter)
        .map(|(front_matter, _)| front_matter)
    else {
        return tags.to_vec();
    };
    let Some(tree_kind) = scan_fm_field(front_matter, "tree_kind") else {
        return tags.to_vec();
    };
    if tree_kind != "source" {
        return tags.to_vec();
    }
    let Some(tree_scope) = scan_fm_field(front_matter, "tree_scope") else {
        return tags.to_vec();
    };

    let source = source_tag(&tree_scope);
    std::iter::once(source.clone())
        .chain(tags.iter().filter(|tag| *tag != &source).cloned())
        .collect()
}

fn write_atomically(abs_path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;

    let parent = abs_path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_path = parent.join(format!(
        ".tmp_sum_tags_{}.md",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp_path)
            .map_err(|error| anyhow::anyhow!("create tag tempfile {:?}: {error}", tmp_path))?;
        file.write_all(bytes)
            .map_err(|error| anyhow::anyhow!("write tag tempfile {:?}: {error}", tmp_path))?;
        file.sync_all()
            .map_err(|error| anyhow::anyhow!("fsync tag tempfile {:?}: {error}", tmp_path))?;
        std::fs::rename(&tmp_path, abs_path).map_err(|error| {
            anyhow::anyhow!(
                "rename tag tempfile {:?} -> {:?}: {error}",
                tmp_path,
                abs_path
            )
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}
