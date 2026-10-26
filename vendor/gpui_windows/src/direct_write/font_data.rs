use std::borrow::Cow;
use windows::{Win32::Graphics::DirectWrite::*, core::*};

#[implement()]
struct FontDataOwner {
    _data: Cow<'static, [u8]>,
}

pub(super) fn create_font_file(
    factory: &IDWriteFactory5,
    loader: &IDWriteInMemoryFontFileLoader,
    data: Cow<'static, [u8]>,
) -> Result<IDWriteFontFile> {
    let size = u32::try_from(data.len())
        .map_err(|_| Error::from_hresult(HRESULT(0x80070057u32 as i32)))?;
    let pointer = data.as_ptr();
    let owner: IUnknown = FontDataOwner { _data: data }.into();
    unsafe { loader.CreateInMemoryFontFileReference(factory, pointer.cast(), size, &owner) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify_reference(data: Cow<'static, [u8]>) -> Result<()> {
        let pointer = data.as_ptr();
        let expected = data.to_vec();
        unsafe {
            let factory: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_ISOLATED)?;
            let loader = factory.CreateInMemoryFontFileLoader()?;
            factory.RegisterFontFileLoader(&loader)?;
            let file = create_font_file(&factory, &loader, data)?;
            let mut key = std::ptr::null_mut();
            let mut key_size = 0;
            file.GetReferenceKey(&mut key, &mut key_size)?;
            let stream = loader.CreateStreamFromKey(key, key_size)?;
            drop(file);
            let mut fragment = std::ptr::null_mut();
            let mut context = std::ptr::null_mut();
            stream.ReadFileFragment(&mut fragment, 0, expected.len() as u64, &mut context)?;
            assert_eq!(fragment.cast::<u8>().cast_const(), pointer);
            assert_eq!(
                std::slice::from_raw_parts(fragment.cast::<u8>(), expected.len()),
                expected
            );
            stream.ReleaseFileFragment(context);
            drop(stream);
            factory.UnregisterFontFileLoader(&loader)?;
        }
        Ok(())
    }

    #[test]
    fn borrowed_font_stream_uses_original_bytes() -> Result<()> {
        verify_reference(Cow::Borrowed(b"static font data"))
    }

    #[test]
    fn owned_font_stream_retains_original_allocation() -> Result<()> {
        verify_reference(Cow::Owned(b"owned font data".to_vec()))
    }

    #[test]
    fn font_owner_is_retained_and_eventually_released() -> Result<()> {
        let data: Cow<'static, [u8]> = Cow::Owned(b"owned font data".to_vec());
        let pointer = data.as_ptr();
        let size = data.len() as u32;
        let owner: IUnknown = FontDataOwner { _data: data }.into();
        let weak = owner.downgrade()?;
        unsafe {
            let factory: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_ISOLATED)?;
            let loader = factory.CreateInMemoryFontFileLoader()?;
            factory.RegisterFontFileLoader(&loader)?;
            let file =
                loader.CreateInMemoryFontFileReference(&factory, pointer.cast(), size, &owner)?;
            drop(owner);
            assert!(weak.upgrade().is_some());
            drop(file);
            factory.UnregisterFontFileLoader(&loader)?;
            drop(loader);
            drop(factory);
        }
        assert!(weak.upgrade().is_none());
        Ok(())
    }
}
