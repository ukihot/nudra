//! RONファイルをそのままアセットとして読む汎用ローダー。
//! 定義データ（Infrastructure）と見た目の定義（Presentation）の両方で使う。

use std::marker::PhantomData;

use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum RonLoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Ron(#[from] ron::error::SpannedError),
}

#[derive(TypePath)]
pub struct RonLoader<A> {
    extensions: &'static [&'static str],
    _asset: PhantomData<fn() -> A>,
}

impl<A: Asset + DeserializeOwned> AssetLoader for RonLoader<A> {
    type Asset = A;
    type Settings = ();
    type Error = RonLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<A, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(ron::de::from_bytes(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        self.extensions
    }
}

/// アセット型とローダーをまとめて登録する。`extensions` は先頭のドットを除いた拡張子。
pub fn register<A: Asset + DeserializeOwned>(app: &mut App, extensions: &'static [&'static str]) {
    app.init_asset::<A>()
        .register_asset_loader(RonLoader::<A> { extensions, _asset: PhantomData });
}
