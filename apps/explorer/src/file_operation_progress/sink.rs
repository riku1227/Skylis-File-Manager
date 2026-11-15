use windows::{
    Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{
            IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl,
            IShellItem, SIGDN_NORMALDISPLAY,
        },
    },
    core::{HRESULT, PCWSTR, Ref, Result, implement},
};

/// Advise and Unadvise live in the same STA as the native operation. No COM pointer escapes.
pub(crate) struct Subscription {
    operation: IFileOperation,
    cookie: u32,
    _sink: IFileOperationProgressSink,
}
impl Subscription {
    pub(crate) fn new(operation: &IFileOperation) -> Result<Self> {
        let sink: IFileOperationProgressSink = Sink.into();
        let cookie = unsafe { operation.Advise(&sink)? };
        Ok(Self {
            operation: operation.clone(),
            cookie,
            _sink: sink,
        })
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        unsafe {
            let _ = self.operation.Unadvise(self.cookie);
        }
    }
}

#[implement(IFileOperationProgressSink)]
struct Sink;
impl Sink_Impl {
    fn current(&self, label: &'static str, item: Ref<'_, IShellItem>) {
        // Display-name lookup stays on the transfer worker, outside the mailbox lock.
        let name = item
            .as_ref()
            .and_then(|item| unsafe {
                let name = item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()?;
                let value = name.to_string().ok();
                CoTaskMemFree(Some(name.0.cast()));
                value
            })
            .map(|name| name.chars().take(1024).collect());
        super::item(label, name);
    }
}
#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn StartOperations(&self) -> Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _: HRESULT) -> Result<()> {
        Ok(())
    }
    fn PreRenameItem(&self, _: u32, item: Ref<'_, IShellItem>, _: &PCWSTR) -> Result<()> {
        self.current("名前の変更", item);
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        _: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> Result<()> {
        self.current("移動", item);
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        _: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> Result<()> {
        self.current("コピー", item);
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        _: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn PreDeleteItem(&self, _: u32, item: Ref<'_, IShellItem>) -> Result<()> {
        self.current("削除", item);
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn PreNewItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        _: &PCWSTR,
        _: u32,
        _: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, total: u32, done: u32) -> Result<()> {
        super::work(total, done);
        Ok(())
    }
    fn ResetTimer(&self) -> Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> Result<()> {
        Ok(())
    }
}
