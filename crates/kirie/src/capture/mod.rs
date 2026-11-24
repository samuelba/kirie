//! Taking screenshots, with the overlay or, for scripts, without it.

mod handoff;
mod interactive;
mod script;

use crate::cli::{CaptureArgs, Mode};

/// Takes the screenshot `kirie capture` asks for.
pub fn run(args: CaptureArgs) -> anyhow::Result<()> {
    match area(&args) {
        Some(area) => {
            wait(args.delay);
            script::capture(
                area,
                &script::Target::of(args.output, args.edit),
                args.cursor,
            )
        }
        None if args.output.is_some() => {
            anyhow::bail!("-o needs `all`, --region or --window: the overlay keeps no file")
        }
        None => interactive::run(args.cursor, args.edit, args.delay),
    }
}

/// The part of the screens a capture takes without the overlay, if it is one of those.
fn area(args: &CaptureArgs) -> Option<script::Area> {
    match (args.region, args.window, args.mode) {
        (Some(region), _, _) => Some(script::Area::Region(region)),
        (None, true, _) => Some(script::Area::Window),
        (None, false, Mode::All) => Some(script::Area::All),
        (None, false, Mode::Region) => None,
    }
}

/// Waits `delay` seconds before a screenshot.
fn wait(delay: u64) {
    if delay > 0 {
        tracing::info!(delay, "waiting before the screenshot");
        std::thread::sleep(std::time::Duration::from_secs(delay));
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;
    use crate::cli::{Cli, Command};

    #[test]
    fn scripts_say_which_part_to_take() {
        let parsed = |args: &[&str]| match Cli::try_parse_from(args).map(|cli| cli.command) {
            Ok(Some(Command::Capture(capture))) => Ok(area(&capture)),
            Ok(other) => panic!("{other:?}"),
            Err(error) => Err(error.kind()),
        };
        assert_eq!(parsed(&["kirie", "capture"]), Ok(None));
        assert_eq!(
            parsed(&["kirie", "capture", "all", "-o", "-"]),
            Ok(Some(script::Area::All))
        );
        assert_eq!(
            parsed(&["kirie", "capture", "--region", "1,2 30x40"]),
            Ok(Some(script::Area::Region(
                kirie_capture::geometry::Rect::new(1, 2, 30, 40)
            )))
        );
        assert_eq!(
            parsed(&["kirie", "capture", "--window", "-o", "w.png"]),
            Ok(Some(script::Area::Window))
        );
        assert_eq!(
            parsed(&["kirie", "capture", "--region", "-1920,0 1920x1080"]),
            Ok(Some(script::Area::Region(
                kirie_capture::geometry::Rect::new(-1920, 0, 1920, 1080)
            ))),
            "screens left of the first one have negative coordinates"
        );
        assert!(parsed(&["kirie", "capture", "--region", "1,2,3"]).is_err());
        assert!(parsed(&["kirie", "capture", "--region", "1,2,3,4", "--window"]).is_err());
        assert_eq!(
            parsed(&["kirie", "capture", "--region", "0,0,200,100", "--edit"]),
            Ok(Some(script::Area::Region(
                kirie_capture::geometry::Rect::new(0, 0, 200, 100)
            )))
        );
        assert_eq!(
            parsed(&["kirie", "capture", "--window", "--edit"]),
            Ok(Some(script::Area::Window))
        );
        assert!(
            parsed(&[
                "kirie", "capture", "--region", "1,2,3,4", "--edit", "-o", "-"
            ])
            .is_err()
        );
    }
}
