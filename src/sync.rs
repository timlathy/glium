use std::sync::mpsc;
use std::ptr;

use Display;

use libc;
use context;
use gl;

/// Provides a way to wait for a server-side operation to be finished.
///
/// Creating a `SyncFence` injects an element in the commands queue of the backend.
/// When this element is reached, the fence becomes signaled.
///
/// ## Example
///
/// ```no_run
/// # let display: glium::Display = unsafe { std::mem::uninitialized() };
/// # fn do_something<T>(_: &T) {}
/// let fence = glium::SyncFence::new_if_supported(&display).unwrap();
/// do_something(&display);
/// fence.wait();   // blocks until the previous operations have finished
/// ```
pub struct SyncFence {
    display: Display,
    id: Option<gl::types::GLsync>,
}

impl SyncFence {
    /// Builds a new `SyncFence` that is injected in the server.
    ///
    /// ## Features
    ///
    /// Only available if the `gl_sync` feature is enabled.
    #[cfg(feature = "gl_sync")]
    pub fn new(display: &Display) -> SyncFence {
        FenceSync::new_if_supported().unwrap()
    }

    /// Builds a new `SyncFence` that is injected in the server.
    ///
    /// Returns `None` is this is not supported by the backend.
    pub fn new_if_supported(display: &Display) -> Option<SyncFence> {
        let (tx, rx) = mpsc::channel();

        display.context.context.exec(move |: mut ctxt| {
            tx.send(unsafe { SyncFencePrototype::new_if_supported(&mut ctxt) }).unwrap();
        });

        rx.recv().unwrap().map(|f| f.into_sync_fence(display))
    }

    /// Blocks until the operation has finished on the server.
    pub fn wait(mut self) {
        let sync = ptr::Unique(self.id.take().unwrap() as *mut libc::c_void);
        let (tx, rx) = mpsc::channel();

        self.display.context.context.exec(move |: ctxt| {
            unsafe {
                // waiting with a deadline of one year
                // the reason why the deadline is so long is because if you attach a GL debugger,
                // the wait can be blocked during a breaking point of the debugger
                let result = ctxt.gl.ClientWaitSync(sync.0 as gl::types::GLsync,
                                                    gl::SYNC_FLUSH_COMMANDS_BIT,
                                                    365 * 24 * 3600 * 1000 * 1000 * 1000);
                tx.send(result).unwrap();
                ctxt.gl.DeleteSync(sync.0 as gl::types::GLsync);
            }
        });

        match rx.recv().unwrap() {
            gl::ALREADY_SIGNALED | gl::CONDITION_SATISFIED => (),
            _ => panic!("Could not wait for the fence")
        };
    }
}

impl Drop for SyncFence {
    fn drop(&mut self) {
        let sync = match self.id {
            None => return,     // fence has already been deleted
            Some(s) => ptr::Unique(s as *mut libc::c_void)
        };

        self.display.context.context.exec(move |: ctxt| {
            unsafe {
                ctxt.gl.DeleteSync(sync.0 as gl::types::GLsync);
            }
        });
    }
}

/// Prototype for a `SyncFence`. Internal type of glium.
///
/// Can be built on the commands queue, then sent to the client and turned into a `SyncFence`.
///
/// The fence must be consumed with either `into_sync_fence` or `wait_and_drop`. Otherwise
/// the destructor will panic.
#[must_use]
pub struct SyncFencePrototype {
    id: Option<gl::types::GLsync>,
}

unsafe impl Send for SyncFencePrototype {}

impl SyncFencePrototype {
    #[cfg(feature = "gl_sync")]
    pub unsafe fn new(ctxt: &mut context::CommandContext) -> SyncFencePrototype {
        ctxt.gl.FenceSync(gl::SYNC_GPU_COMMANDS_COMPLETE, 0)
    }

    pub unsafe fn new_if_supported(ctxt: &mut context::CommandContext) -> Option<SyncFencePrototype> {
        if ctxt.version < &context::GlVersion(3, 2) && !ctxt.extensions.gl_arb_sync {
            return None;
        }

        Some(SyncFencePrototype {
            id: Some(ctxt.gl.FenceSync(gl::SYNC_GPU_COMMANDS_COMPLETE, 0)),
        })
    }

    /// Turns the prototype into a real fence.
    pub fn into_sync_fence(mut self, display: &Display) -> SyncFence {
        SyncFence {
            display: display.clone(),
            id: self.id.take()
        }
    }

    /// Waits for this fence and destroys it, from within the commands context.
    pub unsafe fn wait_and_drop(mut self, ctxt: &mut context::CommandContext) {
        let fence = self.id.take().unwrap();
        ctxt.gl.ClientWaitSync(fence, gl::SYNC_FLUSH_COMMANDS_BIT,
                               365 * 24 * 3600 * 1000 * 1000 * 1000);
        ctxt.gl.DeleteSync(fence);
    }
}

impl Drop for SyncFencePrototype {
    fn drop(&mut self) {
        assert!(self.id.is_none());
    }
}