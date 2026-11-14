use super::*;
use std::{cell::Cell, rc::Rc};

struct HoverProbe(Rc<Cell<usize>>);
impl Render for HoverProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.set(self.0.get() + 1);
        div()
            .size_full()
            .child(div().id("row").w(px(200.)).h(px(28.)).child(row_surface(
                Theme::light(),
                false,
                false,
            )))
    }
}

#[gpui::test]
fn row_hover_repaints_on_enter_and_exit_without_scroll(cx: &mut gpui::TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let counter = renders.clone();
    let window = cx.add_window(move |_, _| HoverProbe(counter));
    let handle = gpui::AnyWindowHandle::from(window);
    cx.update_window(handle, |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(300.), px(100.)), cx);
        w.draw(cx).clear(cx);
    })
    .unwrap();
    let before = renders.get();
    cx.update_window(handle, |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(50.), px(14.)), cx)
    })
    .unwrap();
    assert!(
        renders.get() > before,
        "enter must repaint without scrolling"
    );
    let entered = renders.get();
    cx.update_window(handle, |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(60.), px(14.)), cx)
    })
    .unwrap();
    assert_eq!(
        renders.get(),
        entered,
        "movement inside one row must not redraw continuously"
    );
    cx.update_window(handle, |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(300.), px(100.)), cx)
    })
    .unwrap();
    assert!(
        renders.get() > entered,
        "exit must repaint without scrolling"
    );
}

fn home_window(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<FolderView> {
    // These integration checks use real directory/drive workers, not the deterministic executor.
    cx.background_executor.allow_parking();
    cx.skip_drawing();
    cx.add_window(|w, cx| {
        let icons = cx.new(ShellIcons::new);
        FolderView::new(Location::Home, ThemeMode::Light, icons, w, cx)
    })
}

#[gpui::test]
fn home_does_not_list_a_directory_and_is_reachable_in_history(cx: &mut gpui::TestAppContext) {
    let window = home_window(cx);
    window
        .update(cx, |view, w, cx| {
            assert!(view.is_home());
            assert!(!view.loading);
            assert!(view._listing.is_none());
            let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            view.query = "filter-must-not-follow-navigation".into();
            view.navigate_to(folder.clone(), w, cx);
            assert_eq!(view.path(), Some(folder.as_path()));
            assert!(view.query.is_empty());
            assert!(view.search.read(cx).content.is_empty());
            view.go_back(&GoBack, w, cx);
            assert!(view.is_home());
            assert!(view._listing.is_none());
            view.go_forward(&GoForward, w, cx);
            assert_eq!(view.path(), Some(folder.as_path()));
        })
        .unwrap();
}

#[gpui::test]
fn address_rejects_files_preserves_draft_and_opens_valid_directories(
    cx: &mut gpui::TestAppContext,
) {
    let window = home_window(cx);
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let file = folder.join("Cargo.toml").display().to_string();
    window
        .update(cx, |view, w, cx| {
            view.edit_address(&EditAddress, w, cx);
            view.address
                .update(cx, |input, cx| input.set_text(file.clone(), true, w, cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, cx| view.submit_address(w, cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, cx| {
            assert!(view.is_home());
            assert!(view.editing_address);
            assert!(view.address_error.is_some());
            assert_eq!(view.address.read(cx).content.as_ref(), file);
            view.address.update(cx, |input, cx| {
                input.set_text(folder.display().to_string(), true, w, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, cx| view.submit_address(w, cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert_eq!(view.path(), Some(folder.as_path()));
            assert!(!view.editing_address);
            assert!(view.address_error.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn cancelling_pending_address_cannot_navigate_later(cx: &mut gpui::TestAppContext) {
    let window = home_window(cx);
    window
        .update(cx, |view, w, cx| {
            view.edit_address(&EditAddress, w, cx);
            view.address.update(cx, |input, cx| {
                input.set_text(env!("CARGO_MANIFEST_DIR").into(), true, w, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, cx| {
            view.submit_address(w, cx);
            view.cancel_address(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| assert!(view.is_home()))
        .unwrap();
}
