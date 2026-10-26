//! Actual DirectWrite shaping, without a window, GPU, accounts or network.
use super::{
    DirectWriteState, apply_emoji_keycap_ranges, emoji_keycap_ranges, font_face_to_font,
    slice_from_nullable,
};
use anyhow::Result;
use gpui::FontFallbacks;
use std::{ffi::c_void, path::PathBuf, sync::Arc};
use windows::{Win32::Graphics::DirectWrite::*, core::*};

#[derive(Clone, Debug, PartialEq)]
struct NativeRun {
    family: String,
    position: u32,
    length: u32,
    glyphs: Vec<u16>,
    clusters: Vec<u16>,
}

#[implement(IDWriteTextRenderer)]
struct NativeRecorder {
    runs: Arc<parking_lot::Mutex<Vec<NativeRun>>>,
}

#[allow(non_snake_case)]
impl IDWritePixelSnapping_Impl for NativeRecorder_Impl {
    fn IsPixelSnappingDisabled(&self, _: *const c_void) -> windows::core::Result<BOOL> {
        Ok(BOOL(1))
    }

    fn GetCurrentTransform(
        &self,
        _: *const c_void,
        transform: *mut DWRITE_MATRIX,
    ) -> windows::core::Result<()> {
        unsafe {
            *transform = DWRITE_MATRIX {
                m11: 1.0,
                m22: 1.0,
                ..Default::default()
            };
        }
        Ok(())
    }

    fn GetPixelsPerDip(&self, _: *const c_void) -> windows::core::Result<f32> {
        Ok(1.0)
    }
}

#[allow(non_snake_case)]
impl IDWriteTextRenderer_Impl for NativeRecorder_Impl {
    fn DrawGlyphRun(
        &self,
        _: *const c_void,
        _: f32,
        _: f32,
        _: DWRITE_MEASURING_MODE,
        glyphrun: *const DWRITE_GLYPH_RUN,
        description: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        _: windows::core::Ref<windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        let run = unsafe { &*glyphrun };
        let description = unsafe { &*description };
        let face: IDWriteFontFace3 = run.fontFace.as_ref().unwrap().cast()?;
        let font = font_face_to_font(&face, &HSTRING::from("en-US")).unwrap();
        let glyphs = unsafe {
            slice_from_nullable(run.glyphIndices, run.glyphCount as usize, "null glyphs")?
        }
        .to_vec();
        let clusters = unsafe {
            slice_from_nullable(
                description.clusterMap,
                description.stringLength as usize,
                "null clusters",
            )?
        }
        .to_vec();
        self.runs.lock().push(NativeRun {
            family: font.family.to_string(),
            position: description.textPosition,
            length: description.stringLength,
            glyphs,
            clusters,
        });
        Ok(())
    }

    fn DrawUnderline(
        &self,
        _: *const c_void,
        _: f32,
        _: f32,
        _: *const DWRITE_UNDERLINE,
        _: windows::core::Ref<windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn DrawStrikethrough(
        &self,
        _: *const c_void,
        _: f32,
        _: f32,
        _: *const DWRITE_STRIKETHROUGH,
        _: windows::core::Ref<windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn DrawInlineObject(
        &self,
        _: *const c_void,
        _: f32,
        _: f32,
        _: windows::core::Ref<IDWriteInlineObject>,
        _: BOOL,
        _: BOOL,
        _: windows::core::Ref<windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

fn custom_collection(factory: &IDWriteFactory5) -> Result<IDWriteFontCollection1> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/ui/assets/fonts");
    unsafe {
        let builder = factory.CreateFontSetBuilder()?;
        for name in ["SarasaUiSC-Regular.ttf", "Geist.ttf", "GeistMono.ttf"] {
            let path = HSTRING::from(root.join(name).to_string_lossy().as_ref());
            let file = factory.CreateFontFileReference(&path, None)?;
            builder.AddFontFile(&file)?;
        }
        factory
            .CreateFontCollectionFromFontSet(&builder.CreateFontSet()?)
            .map_err(Into::into)
    }
}

fn shape(
    factory: &IDWriteFactory5,
    custom: &IDWriteFontCollection1,
    system: &IDWriteFontCollection1,
    family: &str,
    text: &str,
    fix_keycaps: bool,
) -> Result<Vec<NativeRun>> {
    let text: Vec<_> = text.encode_utf16().collect();
    unsafe {
        let format: IDWriteTextFormat1 = factory
            .CreateTextFormat(
                &HSTRING::from(family),
                custom,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                15.0,
                &HSTRING::from("zh-CN"),
            )?
            .cast()?;
        let fallbacks = FontFallbacks::from_fonts(vec![
            "Sarasa UI SC".into(),
            "Microsoft YaHei UI".into(),
            "Microsoft YaHei".into(),
            "Noto Sans CJK SC".into(),
        ]);
        if let Some(fallback) =
            DirectWriteState::generate_font_fallbacks(&fallbacks, factory, custom, system)?
        {
            format.SetFontFallback(&fallback)?;
        }
        let layout = factory.CreateTextLayout(&text, &format, 10000.0, 10000.0)?;
        if fix_keycaps {
            apply_emoji_keycap_ranges(&layout, &text, 15.0, system)?;
        }
        let runs = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let recorder: IDWriteTextRenderer = NativeRecorder { runs: runs.clone() }.into();
        layout.Draw(None, &recorder, 0.0, 0.0)?;
        let result = runs.lock().clone();
        Ok(result)
    }
}

#[test]
fn explicit_keycap_ranges_preserve_utf16_offsets_and_text_presentation() {
    let text: Vec<_> = "👩‍💻中A1️⃣B#️⃣C*️⃣ 123 1\u{fe0e}\u{20e3}"
        .encode_utf16()
        .collect();
    let ranges: Vec<_> = emoji_keycap_ranges(&text)
        .map(|range| (range.startPosition, range.length))
        .collect();
    assert_eq!(ranges, vec![(7, 3), (11, 3), (15, 3)]);
    for text in [
        "123",
        "1\u{fe0e}\u{20e3}",
        "1\u{20e3}",
        "a\u{fe0f}\u{20e3}",
        "👩‍💻",
    ] {
        assert!(
            emoji_keycap_ranges(&text.encode_utf16().collect::<Vec<_>>())
                .next()
                .is_none()
        );
    }
}

#[test]
fn native_directwrite_keycaps_form_one_cluster_without_replacing_cjk_or_digits() -> Result<()> {
    unsafe {
        let factory: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let custom = custom_collection(&factory)?;
        let mut system = None;
        factory.GetSystemFontCollection(false, &mut system, true)?;
        let system = system.unwrap();
        for family in ["Sarasa UI SC", "Geist", "Geist Mono"] {
            let text = "中A1️⃣B👩‍💻C123";
            let before = shape(&factory, &custom, &system, family, text, false)?;
            let after = shape(&factory, &custom, &system, family, text, true)?;
            eprintln!("{family} before={before:?}\nafter={after:?}");
            let keycap = after
                .iter()
                .find(|run| run.position <= 2 && run.position + run.length >= 5)
                .expect("keycap must remain in one font run");
            assert_eq!(keycap.family, "Segoe UI Emoji");
            assert_eq!(keycap.position, 2);
            assert_eq!(keycap.length, 3);
            assert_eq!(keycap.glyphs.len(), 1);
            assert_ne!(keycap.glyphs[0], 0);
            assert_eq!(keycap.clusters, [0, 0, 0]);
            let zwj = |runs: &[NativeRun]| {
                runs.iter()
                    .find(|run| run.position == 6 && run.length == 5)
                    .cloned()
            };
            assert_eq!(
                zwj(&before),
                zwj(&after),
                "keycap repair must not change ZWJ fallback"
            );
            let zwj = zwj(&after).expect("Windows must shape the supported ZWJ sample");
            assert_eq!(zwj.family, "Segoe UI Emoji");
            assert_eq!(zwj.glyphs.len(), 1);
            assert_eq!(zwj.clusters, [0, 0, 0, 0, 0]);
            let chinese = after.iter().find(|run| run.position == 0).unwrap();
            assert_eq!(chinese.family, "Sarasa UI SC");
            let digits = after
                .iter()
                .find(|run| run.position <= 12 && run.position + run.length >= 15)
                .unwrap();
            assert_eq!(digits.family, family);
            let text_vs15 = "1\u{fe0e}\u{20e3} 123";
            assert_eq!(
                shape(&factory, &custom, &system, family, text_vs15, false)?,
                shape(&factory, &custom, &system, family, text_vs15, true)?
            );
        }
    }
    Ok(())
}
