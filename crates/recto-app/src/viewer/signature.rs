//! The signature dialog: reuse a saved signature, type one, or draw one on a pad.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, Context, Entity, IntoElement, MouseButton, ParentElement, PathBuilder, Pixels, SharedString,
    Styled, Window, canvas, div, point, prelude::*, px,
};

use super::Viewer;
use super::annotate::Tool;
use crate::i18n::t;
use crate::signatures::{self, DRAWN_ASPECT, Signature};
use crate::text_input::{TextInput, TextInputEvent};

const PAD_HEIGHT: f32 = 120.0;

pub struct SignatureDialog {
    saved: Vec<Signature>,
    typed: Entity<TextInput>,
    /// Strokes drawn on the pad, in pad pixels.
    strokes: Vec<Vec<(f32, f32)>>,
    drawing: bool,
    /// Index in `signatures::INKS`.
    ink: usize,
    /// Where the pad was painted, to turn window positions into pad positions.
    pad: Rc<Cell<Bounds<Pixels>>>,
}

impl Viewer {
    pub(super) fn open_signature_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.theme;
        let typed = cx.new(|cx| TextInput::new(t("Your name"), |_| true, theme, cx));
        cx.subscribe_in(&typed, window, |viewer, _, event, window, cx| match event {
            TextInputEvent::Commit { text, .. } if !text.trim().is_empty() => {
                let ink = viewer.signature_dialog.as_ref().map_or(0, |dialog| dialog.ink);
                viewer.choose_signature(Signature::Typed { name: text.trim().to_owned(), style: 0, ink }, window, cx);
            }
            TextInputEvent::Cancel => viewer.close_signature_dialog(window, cx),
            TextInputEvent::Changed(_) => cx.notify(),
            _ => {}
        })
        .detach();
        self.signature_dialog = Some(SignatureDialog {
            saved: signatures::load(),
            typed,
            strokes: Vec::new(),
            drawing: false,
            ink: 0,
            pad: Rc::new(Cell::new(Bounds::default())),
        });
        cx.notify();
    }

    fn close_signature_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.signature_dialog = None;
        window.focus(&self.focus);
        cx.notify();
    }

    /// Keeps `signature` for the next clicks on pages, and saves it for later documents.
    fn choose_signature(&mut self, signature: Signature, window: &mut Window, cx: &mut Context<Self>) {
        signatures::remember(signature.clone());
        self.signature = Some(signature);
        self.tool = Tool::Signature;
        self.notice = Some(t("Click on a page to place the signature.").into());
        self.close_signature_dialog(window, cx);
    }

    /// The drawn strokes in the unit box `DRAWN_ASPECT` wide, trimmed to their extent.
    fn drawn_signature(dialog: &SignatureDialog) -> Option<Signature> {
        normalize(&dialog.strokes).map(|strokes| Signature::Drawn { strokes, ink: dialog.ink })
    }
}

/// Strokes in pad pixels, scaled into the unit box with their shape kept.
fn normalize(strokes: &[Vec<(f32, f32)>]) -> Option<Vec<Vec<(f32, f32)>>> {
    let points: Vec<(f32, f32)> = strokes.iter().flatten().copied().collect();
    if points.len() < 2 {
        return None;
    }
    let (x0, y0) = points.iter().fold((f32::MAX, f32::MAX), |(x, y), p| (x.min(p.0), y.min(p.1)));
    let (x1, y1) = points.iter().fold((f32::MIN, f32::MIN), |(x, y), p| (x.max(p.0), y.max(p.1)));
    // One scale for both axes, so the signature keeps its shape inside the box.
    let scale = ((x1 - x0) / DRAWN_ASPECT).max(y1 - y0).max(1.0);
    let strokes = strokes
        .iter()
        .map(|stroke| stroke.iter().map(|(x, y)| ((x - x0) / scale / DRAWN_ASPECT, (y - y0) / scale)).collect())
        .collect();
    Some(strokes)
}

impl Viewer {
    pub(super) fn signature_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.signature_dialog.as_ref()?;
        let theme = self.theme;
        let button = |id: SharedString, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px(px(10.))
                .py(px(4.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(primary, |b| b.bg(theme.accent).text_color(theme.on_accent))
                .when(!primary, |b| b.border_1().border_color(theme.border).hover(move |s| s.bg(theme.hover)))
                .child(t(label))
        };
        let saved = dialog.saved.iter().enumerate().map(|(n, signature)| {
            let label: SharedString = match signature {
                Signature::Typed { name, .. } => name.clone().into(),
                Signature::Drawn { strokes, .. } => format!("{} ({} {})", t("Drawn signature"), strokes.len(), t("strokes")).into(),
            };
            let chosen = signature.clone();
            div()
                .id(("saved-signature", n))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(move |s| s.bg(theme.hover))
                .child(label)
                .on_click(cx.listener(move |viewer, _, window, cx| viewer.choose_signature(chosen.clone(), window, cx)))
        });
        let ink = dialog.ink;
        let ink_color = |n: usize| {
            let color = signatures::INKS[n];
            gpui::Rgba { r: color.r, g: color.g, b: color.b, a: 1.0 }
        };
        let name = dialog.typed.read(cx).text().trim().to_owned();
        let mut inks = div().flex().items_center().gap(px(6.)).child(div().text_color(theme.muted).mr(px(4.)).child(t("Ink")));
        for n in 0..signatures::INKS.len() {
            inks = inks.child(
                div()
                    .id(("signature-ink", n))
                    .size(px(20.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .bg(ink_color(n))
                    .border_2()
                    .border_color(if n == ink { theme.highlight } else { theme.toolbar })
                    .on_click(cx.listener(move |viewer, _, _, cx| {
                        if let Some(dialog) = &mut viewer.signature_dialog {
                            dialog.ink = n;
                        }
                        cx.notify();
                    })),
            );
        }
        let mut styles = div().flex().flex_wrap().gap(px(8.));
        for (n, (family, _)) in signatures::STYLES.iter().enumerate() {
            let shown: SharedString = if name.is_empty() { t("Your name").into() } else { name.clone().into() };
            let chosen = name.clone();
            styles = styles.child(
                div()
                    .id(("signature-style", n))
                    .w(px(207.))
                    .h(px(64.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .rounded(px(theme.radius))
                    .border_1()
                    .border_color(theme.border)
                    .bg(gpui::white())
                    .cursor_pointer()
                    .hover(move |s| s.border_color(theme.highlight))
                    .child(div().font_family(*family).text_size(px(28.)).text_color(ink_color(ink)).whitespace_nowrap().child(shown))
                    .on_click(cx.listener(move |viewer, _, window, cx| {
                        if !chosen.is_empty() {
                            viewer.choose_signature(Signature::Typed { name: chosen.clone(), style: n, ink }, window, cx);
                        }
                    })),
            );
        }
        let strokes = dialog.strokes.clone();
        let pad_bounds = dialog.pad.clone();
        let text = ink_color(ink);
        let pad = div()
            .id("signature-pad")
            .w_full()
            .h(px(PAD_HEIGHT))
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.field)
            .cursor_crosshair()
            .child(
                canvas(
                    move |bounds, _, _| pad_bounds.set(bounds),
                    move |bounds, _, window, _| {
                        for stroke in &strokes {
                            let Some(&(x, y)) = stroke.first() else { continue };
                            let mut path = PathBuilder::stroke(px(2.));
                            path.move_to(point(bounds.origin.x + px(x), bounds.origin.y + px(y)));
                            for &(x, y) in &stroke[1..] {
                                path.line_to(point(bounds.origin.x + px(x), bounds.origin.y + px(y)));
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, text);
                            }
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(|viewer, event: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                if let Some(dialog) = &mut viewer.signature_dialog {
                    let origin = dialog.pad.get().origin;
                    dialog.drawing = true;
                    dialog.strokes.push(vec![(f32::from(event.position.x - origin.x), f32::from(event.position.y - origin.y))]);
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|viewer, event: &gpui::MouseMoveEvent, _, cx| {
                if let Some(dialog) = &mut viewer.signature_dialog
                    && dialog.drawing
                {
                    if event.pressed_button != Some(MouseButton::Left) {
                        dialog.drawing = false;
                        return;
                    }
                    let bounds = dialog.pad.get();
                    let x = f32::from(event.position.x - bounds.origin.x).clamp(0.0, f32::from(bounds.size.width));
                    let y = f32::from(event.position.y - bounds.origin.y).clamp(0.0, f32::from(bounds.size.height));
                    if let Some(stroke) = dialog.strokes.last_mut() {
                        stroke.push((x, y));
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|viewer, _, _, _| {
                if let Some(dialog) = &mut viewer.signature_dialog {
                    dialog.drawing = false;
                }
            }));

        let card = div()
            .id("signature-dialog")
            .w(px(470.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .rounded(px(8.))
            .bg(theme.toolbar)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .text_color(theme.text)
            .text_sm()
            .child(div().text_base().child(t("Signature")))
            .when(!dialog.saved.is_empty(), |card| {
                card.child(div().text_color(theme.muted).child(t("Saved signatures"))).children(saved)
            })
            .child(div().text_color(theme.muted).child(t("Type your name, then pick a style")))
            .child(dialog.typed.clone())
            .child(inks)
            .child(styles)
            .child(div().text_color(theme.muted).child(t("Or draw it")))
            .child(pad)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(button("signature-image".into(), "From an image…", false).flex_none().on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.close_signature_dialog(window, cx);
                        viewer.tool = Tool::Image;
                        viewer.notice = Some(t("Click on a page to place the image of your signature.").into());
                        cx.notify();
                    })))
                    .child(div().flex_1())
                    .child(button("signature-clear".into(), "Clear", false).on_click(cx.listener(|viewer, _, _, cx| {
                        if let Some(dialog) = &mut viewer.signature_dialog {
                            dialog.strokes.clear();
                            cx.notify();
                        }
                    })))
                    .child(button("signature-cancel".into(), "Cancel", false).on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.close_signature_dialog(window, cx)
                    })))
                    .child(button("signature-use".into(), "Use drawing", true).on_click(cx.listener(|viewer, _, window, cx| {
                        if let Some(signature) = viewer.signature_dialog.as_ref().and_then(Self::drawn_signature) {
                            viewer.choose_signature(signature, window, cx);
                        }
                    }))),
            );
        Some(
            div()
                .id("signature-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .pt(px(90.))
                .items_start()
                .bg(gpui::rgba(0x0000_0055))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(card)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawings_fit_the_unit_box_keeping_their_shape() {
        let drawn = [vec![(10.0, 20.0), (60.0, 30.0)], vec![(35.0, 40.0)]];
        let Some(strokes) = normalize(&drawn) else { panic!("no drawing") };
        // 50 x 20 pixels: width limits (50 / 2.5 = 20 = height), so both span the box.
        assert_eq!(strokes[0], vec![(0.0, 0.0), (1.0, 0.5)]);
        assert_eq!(strokes[1], vec![(0.5, 1.0)]);
    }
}
