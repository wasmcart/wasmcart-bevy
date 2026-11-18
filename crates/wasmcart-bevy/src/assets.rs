//! Bevy's asset server reading the cart's own asset bundle.
//!
//! The default asset source becomes a reader over `wc_asset_size` /
//! `wc_load_asset`: `asset_server.load("textures/ship.png")` reads
//! `assets/textures/ship.png` from the `.wasc` (or the cart directory). The
//! host validates every path, and a read is synchronous, so a load completes
//! within the frame that polls it. There are no `.meta` files (meta checks are
//! off) and no directory listing.

use std::path::{Path, PathBuf};

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, AssetSourceId, PathStream, Reader,
    VecReader,
};
use bevy::asset::{AssetApp, AssetMetaCheck, AssetPlugin};
use bevy::prelude::*;

/// Reads assets with `wc_load_asset`.
pub struct WasmcartAssetReader;

fn wc_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Read one asset from the cart bundle, or `None` if it does not exist.
pub fn load_bytes(path: &str) -> Option<Vec<u8>> {
    let size = wasmcart::wc_asset_size(path)?;
    let mut buf = vec![0u8; size];
    let n = wasmcart::wc_load_asset(path, &mut buf)?;
    buf.truncate(n);
    Some(buf)
}

impl AssetReader for WasmcartAssetReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        load_bytes(&wc_path(path))
            .map(VecReader::new)
            .ok_or_else(|| AssetReaderError::NotFound(path.to_path_buf()))
    }
    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(PathBuf::from(format!(
            "{}.meta",
            wc_path(path)
        ))))
    }
    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }
    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}

/// Registers [`WasmcartAssetReader`] as the default asset source. Must be
/// added before `AssetPlugin` ([`WasmcartPlugins`](crate::WasmcartPlugins)
/// does).
pub struct WasmcartAssetSourcePlugin;

impl Plugin for WasmcartAssetSourcePlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_source(
            AssetSourceId::Default,
            AssetSourceBuilder::new(|| Box::new(WasmcartAssetReader)),
        );
    }
}

/// `AssetPlugin` settings for a cart: no `.meta` lookups, no processing.
pub fn asset_plugin() -> AssetPlugin {
    AssetPlugin { meta_check: AssetMetaCheck::Never, ..default() }
}
