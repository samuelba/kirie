//! App mode: a window to mark up any image or screenshot, keep it editable, and export it.

mod about;
mod canvas;
mod closing;
mod crop;
mod demo;
mod drop_zone;
mod files;
mod frame;
mod mark_list;
mod release_card;
mod settings;
mod view;
mod viewport;
mod window;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ::image::RgbaImage;
use iced::widget::image;
use iced::{Event, Point, Size, Subscription, Task, event, keyboard};
use kirie_annotate::{Document, Draft, History, Op, Rgba, Shape, Tool, Typesetter};
use kirie_ui::{Palette, Scheme};

use self::closing::{Answer, Closing, Then};
pub use self::files::{Loaded, load};
use self::viewport::Viewport;
use crate::annotate::annotator::{Annotator, Change, Effect};
use crate::annotate::drawing::Drawing;
use crate::annotate::picker;
use crate::annotate::swatches::Swatches;

/// How close to a mark a click picks it, in screen pixels.
const MARK_REACH: f64 = 6.0;

/// The image pixel `at` falls in.
#[allow(clippy::cast_possible_truncation)]
fn pixel(at: kurbo::Point) -> (i64, i64) {
    (at.x.floor() as i64, at.y.floor() as i64)
}

#[derive(Debug, Clone, Copy)]
pub enum Pointer {
    Pressed(kurbo::Point),
    Moved(kurbo::Point),
    Released(kurbo::Point),
}

#[derive(Debug, Clone)]
pub enum Message {
    /// A pointer event in image coordinates, with the current canvas pixels per image pixel.
    Pointer(Pointer, f32),
    Viewport(viewport::Event),
    Crop(crop::Event),
    /// The image point under the pointer, if it is over the canvas.
    Hover(Option<kurbo::Point>),
    /// The canvas has a new size.
    Canvas(Size),
    Modifiers(keyboard::Modifiers),
    PickTool(Tool),
    /// An edit the overlay offers too.
    Annotate(Change),
    Frame(frame::Event),
    Marks(MarksEvent),
    MarkList(mark_list::Event),
    ToggleScheme,
    /// Pastes the clipboard's text into the text being typed.
    PasteText,
    Escape,
    File(files::Event),
    Finish(FinishEvent),
    Outside(OutsideEvent),
    Settings(settings::Event),
    /// Looking for a newer release, and installing it.
    Release(release_card::Event),
    Opened(Result<Loaded, String>),
    ToastExpired(u64),
    Window(window::Event),
    /// The desktop switched between light and dark.
    SystemScheme(Scheme),
}

/// Picking, copying and pasting marks.
#[derive(Debug, Clone)]
pub enum MarksEvent {
    /// Picks every mark with the select tool.
    SelectAll,
    /// Ctrl+C: copies the picked marks, else the picture.
    CopyKey,
    /// Copies the picked marks and deletes them.
    Cut,
    /// Copied marks reached the clipboard, or did not; with what to tell.
    Offered(Result<String, String>),
    /// What was on the clipboard when Ctrl+V was pressed.
    Pasted(Result<Pasted, String>),
}

/// Closing the window, or putting the image away.
#[derive(Debug, Clone, Copy)]
pub enum FinishEvent {
    CloseRequested,
    Close(Answer),
    /// Puts the image away, asking first about unsaved changes.
    Discard,
}

/// The about card, and links and files handed to the desktop.
#[derive(Debug, Clone)]
pub enum OutsideEvent {
    /// Opens or closes the about card.
    About(bool),
    OpenLink(&'static str),
    /// Shows a saved file in the file manager.
    ShowInFiles(PathBuf),
}

/// How far copied marks got to the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Offer {
    Pending,
    Failed,
}

/// What Ctrl+V found on the clipboard.
#[derive(Debug, Clone)]
pub enum Pasted {
    Marks(Vec<Shape>),
    Image(RgbaImage),
}

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    text: String,
    error: bool,
    /// The file just saved, to show in the file manager.
    file: Option<PathBuf>,
}

pub struct Editor {
    /// The window, unless it is closed while a screenshot is taken.
    window: Option<iced::window::Id>,
    placement: window::Placement,
    palette: Palette,
    image: Option<(RgbaImage, image::Handle)>,
    path: Option<PathBuf>,
    annotator: Annotator,
    /// The document as last opened, saved or copied, to tell unsaved changes.
    kept: Document,
    closing: Closing,
    /// The about card is open.
    about: bool,
    /// The settings card, while it is open.
    settings: Option<settings::Form>,
    /// Looking for newer releases, and what was found.
    updates: release_card::Updates,
    /// What to do once the question about unsaved changes is answered.
    then: Then,
    viewport: Viewport,
    cropping: Option<crop::Cropping>,
    /// The canvas's size, for the zoom level when the image is fitted.
    canvas: Size,
    /// The image point under the pointer.
    hover: Option<kurbo::Point>,
    tool: Tool,
    mark_list: mark_list::Shown,
    /// The marks copied last, while the clipboard is not known to hold them: Ctrl+V pastes them
    /// while they are on their way there, or, should they never get there, when it holds
    /// neither marks nor an image.
    clip: Option<(Vec<Shape>, Offer)>,
    toast: Option<Toast>,
    toasts: u64,
    /// Set once the user picks light or dark; until then the editor follows the desktop.
    scheme_chosen: bool,
    typesetter: RefCell<Typesetter>,
    patches: RefCell<HashMap<crate::annotate::paint::PatchKey, image::Handle>>,
    pictures: RefCell<HashMap<u64, image::Handle>>,
    drops: files::Drops,
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor")
            .field("path", &self.path)
            .field("tool", &self.tool)
            .finish_non_exhaustive()
    }
}

impl Editor {
    #[cfg(test)]
    pub fn boot(loaded: Option<Loaded>) -> (Self, Task<Message>) {
        Self::boot_with(loaded, None)
    }

    /// Like [`Self::boot`], bringing the window to the front with an activation `token`.
    fn boot_with(loaded: Option<Loaded>, token: Option<String>) -> (Self, Task<Message>) {
        let swatches = Swatches::load();
        let drawing = Drawing::load();
        let paint = drawing.paint_for(Some(Tool::Arrow), &swatches);
        let blank = drawing.styled(Document::new(1, 1, 1.0));
        let placement = window::Placement::default();
        let (id, opened) = iced::window::open(window::settings(placement));
        let mut editor = Self {
            window: Some(id),
            placement,
            palette: Palette::DARK,
            image: None,
            path: None,
            annotator: Annotator::new(blank.clone(), paint, swatches),
            kept: blank,
            closing: Closing::No,
            about: false,
            settings: None,
            updates: release_card::Updates::new(),
            then: Then::Exit,
            viewport: Viewport::fitted(),
            cropping: None,
            canvas: Size::ZERO,
            hover: None,
            tool: Tool::Arrow,
            mark_list: mark_list::Shown::Closed,
            clip: None,
            toast: None,
            toasts: 0,
            scheme_chosen: false,
            typesetter: RefCell::new(Typesetter::new()),
            patches: RefCell::new(HashMap::new()),
            pictures: RefCell::new(HashMap::new()),
            drops: files::Drops::default(),
        };
        if let Some(loaded) = loaded {
            editor.open(loaded);
        }
        let opened = opened.then(move |id| {
            let raised = token
                .clone()
                .map_or_else(Task::none, |token| window::activate(id, token).discard());
            Task::batch([raised, files::take_drops(id)])
        });
        (editor, opened)
    }

    fn open(&mut self, mut loaded: Loaded) {
        let handoff = loaded
            .path
            .take_if(|path| crate::storage::save::is_handoff(path))
            .map(std::fs::remove_file)
            .is_some();
        let handle = image::Handle::from_rgba(
            loaded.image.width(),
            loaded.image.height(),
            loaded.image.as_raw().clone(),
        );
        // The overlay may have changed the options since, in a process of its own.
        let drawing = Drawing::load();
        self.reload_shared();
        let demo = (loaded.fresh || handoff) && demo::wanted();
        let document = match (loaded.fresh, handoff) {
            (true, _) => drawing.framed(drawing.styled(loaded.document)),
            (false, true) => drawing.framed(loaded.document),
            (false, false) => loaded.document,
        };
        let document = if demo {
            demo::marked(document)
        } else {
            document
        };
        self.annotator.paint = drawing.paint_for(Some(self.tool), &self.annotator.swatches);
        self.kept = document.clone();
        self.annotator.history = History::new(document);
        self.annotator.follow_style(drawing.style);
        self.image = Some((loaded.image, handle));
        self.path = loaded.path;
        self.viewport = Viewport::fitted();
        self.cropping = None;
        self.annotator.draft = None;
        self.annotator.picked.clear();
        if demo {
            self.tool = Tool::Select;
            self.annotator.picked = demo::picked(self.document()).into_iter().collect();
        }
        self.annotator.typing = None;
        self.annotator.sliding = false;
        self.closing = Closing::No;
        self.then = Then::Exit;
        self.patches.borrow_mut().clear();
    }

    /// Reads the colors and recent emoji the overlay may have saved, so the editor's next save
    /// does not put back its older copy.
    fn reload_shared(&mut self) {
        self.annotator.swatches = Swatches::load();
        self.annotator.emojis.reload();
    }

    /// Puts the image away: the editor shows its empty page again.
    fn clear(&mut self) {
        let blank = Drawing::load().styled(Document::new(1, 1, 1.0));
        self.kept = blank.clone();
        self.annotator.history = History::new(blank);
        self.image = None;
        self.path = None;
        self.viewport = Viewport::fitted();
        self.cropping = None;
        self.hover = None;
        self.annotator.draft = None;
        self.annotator.picked.clear();
        self.annotator.typing = None;
        self.annotator.sliding = false;
        self.closing = Closing::No;
        self.then = Then::Exit;
        self.patches.borrow_mut().clear();
    }

    fn document(&self) -> &Document {
        self.annotator.history.current()
    }

    fn image_size(&self) -> Size {
        self.image.as_ref().map_or(Size::ZERO, |(image, _)| {
            #[allow(clippy::cast_precision_loss)]
            Size::new(image.width() as f32, image.height() as f32)
        })
    }

    /// What the canvas shows, in image coordinates: all of the image while cropping, else
    /// what gets saved, the kept part in its frame.
    fn content(&self) -> kurbo::Rect {
        if self.cropping.is_some() {
            let size = self.image_size();
            kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height))
        } else {
            self.document().canvas()
        }
    }

    /// What the canvas draws on: like [`Self::content`], but grown as marks being drawn reach
    /// beyond it, before they are done and the view settles on them.
    fn shown_canvas(&self) -> kurbo::Rect {
        if self.cropping.is_some() {
            self.content()
        } else {
            self.document().canvas_with(&self.drafts())
        }
    }

    /// The marks being drawn, moved, reshaped or typed, as they will be once done.
    fn drafts(&self) -> Vec<Shape> {
        let document = self.document();
        let mut drafts = Vec::new();
        match &self.annotator.draft {
            Some(Draft::Moving { ids, by, .. }) => {
                drafts.extend(
                    ids.iter()
                        .filter_map(|id| document.shape(*id))
                        .map(|shape| Shape {
                            kind: shape.kind.translated(*by),
                            ..shape.clone()
                        }),
                );
            }
            Some(draft) => {
                drafts.extend(draft.edited().cloned());
                drafts.extend(draft.preview().map(|kind| Shape {
                    id: document.next_id(),
                    kind,
                    paint: self.annotator.paint,
                    style: None,
                    rotation: 0.0,
                }));
            }
            None => {}
        }
        drafts.extend(
            self.annotator
                .typing
                .as_ref()
                .map(|typing| typing.preview(document, self.annotator.paint)),
        );
        drafts
    }

    pub fn title(&self, _: iced::window::Id) -> String {
        match self.path.as_deref().and_then(Path::file_name) {
            Some(name) => format!("{} — Kirie", name.to_string_lossy()),
            None => "Kirie".to_owned(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pointer(pointer, scale) => return self.pointer(pointer, scale),
            // Pans come with every pointer move, and need no content worked out.
            Message::Viewport(viewport::Event::Pan(by)) => self.viewport = self.viewport.panned(by),
            Message::Viewport(event) => {
                self.viewport = self.viewport.update(event, self.content(), self.canvas);
            }
            Message::Hover(at) => self.hover = at,
            Message::Canvas(size) => self.canvas = size,
            Message::Modifiers(modifiers) => self.annotator.modifiers = modifiers,
            Message::PickTool(tool) => self.pick_tool(tool),
            Message::Annotate(change) => {
                if matches!(change, Change::Undo | Change::Redo) {
                    self.cropping = None;
                }
                let effect = self.annotator.update(change);
                return self.annotated(effect);
            }
            Message::Frame(event) => self.frame(event),
            Message::SystemScheme(scheme) => self.follow(Some(scheme)),
            Message::ToggleScheme => self.follow(None),
            Message::PasteText => return self.paste_text(),
            Message::Marks(event) => return self.edit_marks(event),
            Message::MarkList(event) => return self.mark_list_event(event),
            Message::Escape if self.closing == Closing::Asking => self.closing = Closing::No,
            Message::Escape | Message::Outside(OutsideEvent::About(false)) if self.about => {
                self.about = false;
            }
            Message::Escape => return self.escape(),
            Message::Settings(event) => return self.edit_settings(event),
            Message::Release(event) => return self.release_event(event),
            Message::Outside(event) => return self.outside(event),
            Message::Crop(event) => self.crop_event(event),
            Message::File(event) => return self.files(event),
            Message::Finish(event) => return self.finish(event),
            Message::Opened(Ok(loaded)) => return self.replace(loaded),
            Message::Opened(Err(error)) => return self.toast(Err(error)),
            Message::Window(event) => return self.window_event(event),
            Message::ToastExpired(id) => {
                if self.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    self.toast = None;
                }
            }
        }
        Task::none()
    }

    fn pick_tool(&mut self, tool: Tool) {
        self.keep_crop();
        self.annotator.pick_tool(Some(tool));
        self.tool = tool;
    }

    /// Acts on what an edit did: the text box takes the keyboard, the drawing options are kept
    /// for the next time, and colors that could not be kept are told.
    fn annotated(&mut self, effect: Effect) -> Task<Message> {
        if effect.settled {
            self.remember_drawing();
        }
        let focus = if effect.focus {
            iced::widget::operation::focus(view::TEXT_INPUT)
        } else {
            Task::none()
        };
        match effect.error {
            Some(error) => Task::batch([focus, self.toast(Err(error))]),
            None => focus,
        }
    }

    /// Changes to the frame around the picture, kept for the next time unless a slider is
    /// still dragged.
    fn frame(&mut self, event: frame::Event) {
        let document = self.document();
        let op = Op::SetFrame(event.apply(document.frame, document.unit));
        match event {
            frame::Event::MarginDone => self.annotator.sliding = false,
            frame::Event::Margin(_) => {
                self.annotator.adjust(op, true);
                return;
            }
            frame::Event::Background(_)
            | frame::Event::ToggleShadow
            | frame::Event::ToggleOverflow => {
                self.annotator.history.apply(op);
            }
        }
        self.remember_drawing();
    }

    /// Esc puts down what is open, one thing at a time: the eyedropper, the settings, the crop,
    /// the color picker, the text being typed, the mark being drawn, then the picked marks.
    fn escape(&mut self) -> Task<Message> {
        if self.annotator.sampling() {
            let effect = self.annotator.edit_color(picker::Event::Eyedropper);
            return self.annotated(effect);
        }
        if self.settings.take().is_some() {
            return Task::none();
        }
        if self.cropping.take().is_some() {
            self.viewport = Viewport::fitted();
            return Task::none();
        }
        match self.annotator.put_down() {
            Some(effect) => self.annotated(effect),
            None => Task::none(),
        }
    }

    /// Picking, copying and pasting marks.
    fn edit_marks(&mut self, event: MarksEvent) -> Task<Message> {
        match event {
            MarksEvent::CopyKey if self.annotator.picked.is_empty() => {
                return self.export(files::Export::Copy);
            }
            MarksEvent::CopyKey => return self.copy_marks(),
            MarksEvent::Cut if !self.annotator.picked.is_empty() => {
                let copied = self.copy_marks();
                let ids = std::mem::take(&mut self.annotator.picked);
                self.annotator.history.apply(Op::Remove { ids });
                return copied;
            }
            MarksEvent::Pasted(Ok(Pasted::Marks(_))) if self.image.is_none() => {
                return self.toast(Err("Open an image to paste the marks on".to_owned()));
            }
            MarksEvent::Pasted(Ok(Pasted::Marks(clip))) => {
                self.keep_crop();
                let _ = self.update(Message::PickTool(Tool::Select));
                if let Some((op, ids)) = kirie_annotate::marks::paste(self.document(), &clip) {
                    self.annotator.history.apply(op);
                    self.annotator.picked = ids;
                }
            }
            MarksEvent::Pasted(Ok(Pasted::Image(image))) if self.image.is_none() => {
                return self.update(Message::Opened(Ok(Loaded::image(image, None))));
            }
            MarksEvent::Pasted(Ok(Pasted::Image(image))) => {
                self.keep_crop();
                match kirie_annotate::marks::place_image(self.document(), image) {
                    Ok((op, id)) => {
                        let _ = self.update(Message::PickTool(Tool::Select));
                        self.annotator.history.apply(op);
                        self.annotator.picked = vec![id];
                    }
                    Err(error) => return self.toast(Err(error)),
                }
            }
            MarksEvent::Pasted(Err(error)) => return self.toast(Err(error)),
            MarksEvent::Offered(Ok(copied)) => {
                self.clip = None;
                return self.toast(Ok(copied));
            }
            MarksEvent::Offered(Err(copied)) => {
                if let Some((_, offer)) = &mut self.clip {
                    *offer = Offer::Failed;
                }
                return self.toast(Ok(copied));
            }
            MarksEvent::SelectAll if self.image.is_some() && self.cropping.is_none() => {
                let _ = self.update(Message::PickTool(Tool::Select));
                let document = self.document();
                let canvas = document.canvas();
                self.annotator.picked = document
                    .shapes()
                    .filter(|shape| {
                        let area = shape.turn().transform_rect_bbox(document.bounds(shape));
                        area.x0 <= canvas.x1
                            && canvas.x0 <= area.x1
                            && area.y0 <= canvas.y1
                            && canvas.y0 <= area.y1
                    })
                    .map(|shape| shape.id)
                    .collect();
            }
            MarksEvent::Cut | MarksEvent::SelectAll => {}
        }
        Task::none()
    }

    /// Puts the picked marks on the clipboard, for Ctrl+V here or in another Kirie.
    fn copy_marks(&mut self) -> Task<Message> {
        let marks = kirie_annotate::marks::of(self.document(), &self.annotator.picked);
        if marks.is_empty() {
            return Task::none();
        }
        self.clip = Some((marks.clone(), Offer::Pending));
        let copied = if marks.len() == 1 {
            "Copied the mark; Ctrl+V pastes it".to_owned()
        } else {
            format!("Copied {} marks; Ctrl+V pastes them", marks.len())
        };
        let device = self.drops.device.clone();
        let offered = async move {
            let bytes = tokio::task::spawn_blocking(move || kirie_annotate::marks::encode(marks))
                .await
                .map_err(|error| error.to_string())??;
            crate::desktop::clipboard::offer_from(bytes, kirie_annotate::marks::MIME, device).await
        };
        Task::perform(offered, move |offered| {
            Message::Marks(MarksEvent::Offered(match offered {
                Ok(()) => Ok(copied),
                Err(error) => Err(format!("{copied} here; other apps cannot ({error})")),
            }))
        })
    }

    /// Closing the window, or putting the image away.
    fn finish(&mut self, event: FinishEvent) -> Task<Message> {
        match event {
            FinishEvent::CloseRequested => self.close_requested(),
            FinishEvent::Discard => self.discard(),
            FinishEvent::Close(answer) => self.answer(answer),
        }
    }

    /// Keeps the drawing options for the next image, and the next session.
    fn remember_drawing(&self) {
        let drawing = Drawing::load().with_marks(self.annotator.paint, self.document());
        if self.image.is_some() {
            drawing.with_frame(self.document()).save();
        } else {
            drawing.save();
        }
    }

    /// Follows the desktop's `scheme` until the user switches it themselves (`None`).
    fn follow(&mut self, scheme: Option<Scheme>) {
        match scheme {
            Some(scheme) if !self.scheme_chosen => self.palette = Palette::of(scheme),
            Some(_) => {}
            None => {
                self.scheme_chosen = true;
                self.palette = match self.palette.scheme {
                    Scheme::Dark => Palette::LIGHT,
                    Scheme::Light => Palette::DARK,
                };
            }
        }
    }

    fn pointer(&mut self, pointer: Pointer, scale: f32) -> Task<Message> {
        if self.image.is_none() {
            return Task::none();
        }
        if let Pointer::Moved(at) = pointer {
            self.hover = Some(at);
        }
        if self.annotator.sampling() {
            return match pointer {
                Pointer::Pressed(at) => self.sample(at),
                _ => Task::none(),
            };
        }
        if self.cropping.is_some() {
            self.crop_pointer(pointer, scale);
            return Task::none();
        }
        let effect = match pointer {
            Pointer::Pressed(at) => {
                let reach = MARK_REACH / f64::from(scale.max(0.01));
                self.annotator
                    .press(self.tool, at, on_screen(at, scale), reach)
            }
            Pointer::Moved(at) => {
                self.annotator.drag(at);
                return Task::none();
            }
            Pointer::Released(at) => self.annotator.release(at),
        };
        self.annotated(effect)
    }

    /// Hands the picker the image's color at `at`, if `at` is on the image.
    fn sample(&mut self, at: kurbo::Point) -> Task<Message> {
        let (column, row) = pixel(at);
        let color = self.image.as_ref().and_then(|(image, _)| {
            let found =
                image.get_pixel_checked(u32::try_from(column).ok()?, u32::try_from(row).ok()?)?;
            let [red, green, blue, _] = found.0;
            Some(Rgba::rgb(red, green, blue))
        });
        match color {
            Some(color) => {
                let effect = self.annotator.edit_color(picker::Event::Sampled(color));
                self.annotated(effect)
            }
            None => Task::none(),
        }
    }

    /// The about card, and links and files handed to the desktop.
    fn outside(&mut self, event: OutsideEvent) -> Task<Message> {
        match event {
            OutsideEvent::About(open) => {
                self.about = open;
                if open {
                    return self.check_on_about();
                }
            }
            OutsideEvent::OpenLink(link) => crate::desktop::launch::open(link),
            OutsideEvent::ShowInFiles(path) => {
                return Task::future(
                    async move { crate::desktop::launch::show_in_files(&path).await },
                )
                .discard();
            }
        }
        Task::none()
    }

    /// Pastes text from the clipboard, read through the window's data device, else through
    /// iced's clipboard.
    fn paste_text(&self) -> Task<Message> {
        let paste = |text: String| {
            Message::Annotate(Change::TextEdit(iced::widget::text_editor::Action::Edit(
                iced::widget::text_editor::Edit::Paste(std::sync::Arc::new(text)),
            )))
        };
        let nothing = || Message::File(files::Event::Done(Ok(String::new())));
        Task::perform(
            crate::desktop::drops::clipboard(
                self.drops.device.clone(),
                &crate::desktop::drops::TEXT,
            ),
            |bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
        )
        .then(move |text| match text {
            Some(text) => Task::done(paste(text)),
            None => iced::clipboard::read().map(move |text| text.map_or_else(nothing, paste)),
        })
    }

    fn toast(&mut self, result: Result<String, String>) -> Task<Message> {
        match result {
            Ok(text) => self.show_toast(text, false, None),
            Err(text) => self.show_toast(text, true, None),
        }
    }

    /// Tells that the image was saved to `file`, or copied without one.
    fn toast_file(&mut self, text: String, file: Option<PathBuf>) -> Task<Message> {
        self.show_toast(text, false, file)
    }

    fn show_toast(&mut self, text: String, error: bool, file: Option<PathBuf>) -> Task<Message> {
        self.toasts += 1;
        let id = self.toasts;
        // A toast with a button stays a little longer, to reach the button.
        let shown = Duration::from_secs(if file.is_some() { 8 } else { 4 });
        self.toast = Some(Toast {
            id,
            text,
            error,
            file,
        });
        Task::perform(tokio::time::sleep(shown), move |()| {
            Message::ToastExpired(id)
        })
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let quiet = self.annotator.typing.is_some()
            || self.annotator.picker.is_some()
            || self.closing != Closing::No
            || self.about
            || self.settings.is_some();
        let keys = if quiet {
            event::listen_with(typing_key)
        } else if self.cropping.is_some() {
            event::listen_with(crop::key)
        } else {
            event::listen_with(key)
        };
        let releases = if self.updates.check() {
            iced::time::every(release_card::TICK)
                .map(|_| Message::Release(release_card::Event::Tick))
        } else {
            Subscription::none()
        };
        Subscription::batch([
            keys,
            releases,
            event::listen_with(window::focus),
            Subscription::run(crate::desktop::ipc::requests)
                .map(|value| Message::Window(window::Event::Ipc(value))),
            Subscription::run(crate::desktop::appearance::changes).map(Message::SystemScheme),
            iced::window::close_requests().map(|_| Message::Finish(FinishEvent::CloseRequested)),
        ])
    }

    pub fn theme(&self, _: iced::window::Id) -> iced::Theme {
        match self.palette.scheme {
            Scheme::Dark => iced::Theme::Dark,
            Scheme::Light => iced::Theme::Light,
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn typing_key(event: Event, _: event::Status, _: iced::window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
            Some(Message::Modifiers(modifiers))
        }
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        }) => Some(Message::Escape),
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            quit_key(&key, modifiers)
        }
        _ => None,
    }
}

/// Ctrl+Q closes the editor as its window's close button does, asking about unsaved changes.
fn quit_key(key: &keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    (modifiers.command() && key.as_ref() == keyboard::Key::Character("q"))
        .then_some(Message::Finish(FinishEvent::CloseRequested))
}

#[allow(clippy::cast_possible_truncation)]
fn to_point(at: kurbo::Point) -> Point {
    Point::new(at.x as f32, at.y as f32)
}

/// The image point `at` as far apart from others as the picture shows it at `scale`, so that a
/// double click allows the same slip at every zoom.
fn on_screen(at: kurbo::Point, scale: f32) -> Point {
    to_point((at.to_vec2() * f64::from(scale)).to_point())
}

#[allow(clippy::needless_pass_by_value)]
fn key(event: Event, status: event::Status, _: iced::window::Id) -> Option<Message> {
    use keyboard::key::Named;
    if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
        return Some(Message::Modifiers(modifiers));
    }
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };
    if let Some(quit) = quit_key(&key, modifiers) {
        return Some(quit);
    }
    if status == event::Status::Captured {
        return None;
    }
    let command = modifiers.command();
    if command
        && let keyboard::Key::Character(written) = key.as_ref()
        && let Some(to) = kirie_annotate::marks::restack_key(written)
    {
        return Some(Message::Annotate(Change::Restack(to)));
    }
    match key.as_ref() {
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Named(Named::Enter) if modifiers.shift() || command => {
            Some(Message::File(files::Event::Save))
        }
        keyboard::Key::Named(Named::Delete | Named::Backspace) => {
            Some(Message::Annotate(Change::Delete))
        }
        keyboard::Key::Character("z") if command && modifiers.shift() => {
            Some(Message::Annotate(Change::Redo))
        }
        keyboard::Key::Character("z") if command => Some(Message::Annotate(Change::Undo)),
        keyboard::Key::Character("y") if command => Some(Message::Annotate(Change::Redo)),
        keyboard::Key::Character("c") if command => Some(Message::Marks(MarksEvent::CopyKey)),
        keyboard::Key::Character("x") if command => Some(Message::Marks(MarksEvent::Cut)),
        keyboard::Key::Character("d") if command => Some(Message::Annotate(Change::Duplicate)),
        keyboard::Key::Character("v") if command => Some(Message::File(files::Event::Paste)),
        keyboard::Key::Character("w") if command => Some(Message::Finish(FinishEvent::Discard)),
        keyboard::Key::Character("a") if command => Some(Message::Marks(MarksEvent::SelectAll)),
        keyboard::Key::Character(",") if command => Some(Message::Settings(settings::Event::Open)),
        keyboard::Key::Character("o") if command => Some(Message::File(files::Event::Open)),
        keyboard::Key::Character("s") if command && modifiers.shift() => {
            Some(Message::File(files::Event::SaveAs))
        }
        keyboard::Key::Character("s") if command && modifiers.alt() => {
            Some(Message::File(files::Event::SaveProject))
        }
        keyboard::Key::Character("s") if command => Some(Message::File(files::Event::Save)),
        keyboard::Key::Character("0") if command => Some(Message::Viewport(viewport::Event::Fit)),
        keyboard::Key::Character("1") if command => {
            Some(Message::Viewport(viewport::Event::Actual))
        }
        keyboard::Key::Character("=" | "+") if command => {
            Some(Message::Viewport(viewport::Event::ZoomIn))
        }
        keyboard::Key::Character("-") if command => {
            Some(Message::Viewport(viewport::Event::ZoomOut))
        }
        keyboard::Key::Character("c" | "C") if !command && modifiers.shift() => {
            Some(Message::Crop(crop::Event::Start))
        }
        keyboard::Key::Character("m") if !command => {
            Some(Message::MarkList(mark_list::Event::Toggle))
        }
        keyboard::Key::Character(c) if !command => c
            .chars()
            .next()
            .and_then(Tool::from_key)
            .map(Message::PickTool),
        _ => None,
    }
}

/// A card the editor opens with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    About,
    Settings,
}

/// Opens the editor window, with `loaded` in it if given, and `card` if given; the activation
/// `token` brings it to the front.
pub fn run(loaded: Option<Loaded>, card: Option<Card>, token: Option<String>) -> iced::Result {
    let loaded = std::sync::Mutex::new(loaded);
    iced::daemon(
        move || {
            let (mut editor, task) = Editor::boot_with(
                loaded.lock().ok().and_then(|mut loaded| loaded.take()),
                token.clone(),
            );
            match card {
                Some(Card::About) => editor.about = true,
                Some(Card::Settings) => {
                    let _ = editor.edit_settings(settings::Event::Open);
                }
                None => {}
            }
            let checked = if editor.updates.check() {
                editor.check_release(false)
            } else {
                Task::none()
            };
            (editor, Task::batch([task, checked]))
        },
        Editor::update,
        Editor::view,
    )
    .antialiasing(true)
    .font(kirie_annotate::text::EMOJI_FONT)
    .title(Editor::title)
    .theme(Editor::theme)
    .subscription(Editor::subscription)
    .run()
}

/// `change` as the message the views send for it.
fn annotate<T>(change: impl Fn(T) -> Change + Clone) -> impl Fn(T) -> Message + Clone {
    move |value| Message::Annotate(change(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: &str, modifiers: keyboard::Modifiers) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character(key.into()),
            modified_key: keyboard::Key::Character(key.into()),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn ctrl_q_closes_the_editor_even_while_typing() {
        let id = iced::window::Id::unique();
        let ctrl_q = || press("q", keyboard::Modifiers::CTRL);
        for listen in [key, typing_key, crop::key] {
            assert!(matches!(
                listen(ctrl_q(), event::Status::Ignored, id),
                Some(Message::Finish(FinishEvent::CloseRequested))
            ));
        }
        assert!(matches!(
            key(ctrl_q(), event::Status::Captured, id),
            Some(Message::Finish(FinishEvent::CloseRequested))
        ));
        assert!(
            typing_key(
                press("q", keyboard::Modifiers::empty()),
                event::Status::Ignored,
                id
            )
            .is_none()
        );
    }

    #[test]
    fn every_tool_key_picks_its_tool_and_shift_c_crops() {
        let id = iced::window::Id::unique();
        for tool in Tool::ALL {
            let letter = tool.key().to_string();
            assert!(
                matches!(
                    key(press(&letter, keyboard::Modifiers::empty()), event::Status::Ignored, id),
                    Some(Message::PickTool(picked)) if picked == tool
                ),
                "{letter} picks {tool:?}"
            );
        }
        for written in ["c", "C"] {
            assert!(matches!(
                key(
                    press(written, keyboard::Modifiers::SHIFT),
                    event::Status::Ignored,
                    id
                ),
                Some(Message::Crop(crop::Event::Start))
            ));
        }
    }

    #[test]
    fn a_double_click_allows_the_same_slip_on_screen_at_every_zoom() {
        let first = kurbo::Point::new(100.0, 100.0);
        let second = kurbo::Point::new(108.0, 100.0);
        for (scale, double) in [(0.25, true), (4.0, false)] {
            let mut last = None;
            assert!(!crate::annotate::typing::double_click(
                &mut last,
                on_screen(first, scale)
            ));
            assert_eq!(
                crate::annotate::typing::double_click(&mut last, on_screen(second, scale)),
                double,
                "8 image px apart at {scale}x"
            );
        }
    }
}
