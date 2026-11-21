//! The in-app video player decodes through a real `ffmpeg` pipe. Every test makes its own clip
//! with `ffmpeg`, so nothing depends on a checked-in file; without the tools they skip.

#[cfg(unix)]
mod unix {
    use rustsidian_app::video::{find_program, FfmpegPipe, Playback, State, VideoPlayer};
    use rustsidian_app::{render, App};
    use rustsidian_ui::{Cmd, Fonts, Pixmap, Rect};
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const CAP: (u32, u32) = (640, 360);

    fn have_tools() -> bool {
        find_program("ffmpeg", std::env::var_os("PATH")).is_some()
            && find_program("ffprobe", std::env::var_os("PATH")).is_some()
    }

    /// A one second 160x120 clip at 10 fps, in a vault of its own.
    fn clip() -> Option<tempfile::TempDir> {
        if !have_tools() {
            eprintln!("skipping: ffmpeg and ffprobe are not on PATH");
            return None;
        }
        let d = tempfile::tempdir().unwrap();
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=160x120:rate=10",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(d.path().join("clip.mp4"))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(made, "ffmpeg could not make the fixture clip");
        Some(d)
    }

    fn open(v: &mut VideoPlayer, dir: &Path) {
        v.open("clip.mp4", &dir.join("clip.mp4"), "Note.md", CAP).expect("the clip opens");
    }

    /// Tick until a frame is in hand, or five seconds pass.
    fn wait_frame(v: &mut VideoPlayer) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            v.tick(Instant::now());
            if v.current().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Keep ticking for `ms`, so anything the decoder still had queued arrives.
    fn pump(v: &mut VideoPlayer, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            v.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Pids whose command line mentions `dir`: the decoders running on this clip.
    fn decoders_of(dir: &Path) -> Vec<u32> {
        let needle = dir.to_string_lossy().to_string();
        let mut pids = Vec::new();
        let Ok(entries) = fs::read_dir("/proc") else { return pids };
        for entry in entries.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(cmdline) = fs::read(entry.path().join("cmdline")) else { continue };
            if String::from_utf8_lossy(&cmdline).contains(&needle) {
                pids.push(pid);
            }
        }
        pids
    }

    #[test]
    fn open_probes_the_file_and_starts_a_decoder() {
        let Some(d) = clip() else { return };
        let mut v = VideoPlayer::default();
        open(&mut v, d.path());
        assert!(v.is_open());
        assert_eq!(v.path(), "clip.mp4");
        assert_eq!(v.note(), "Note.md");
        assert_eq!(v.state(), &State::Loading, "the first frame promotes it to Playing",);
        assert!(v.is_running(), "the decoder is not running");
        // The container says one second.
        assert!((v.duration() - 1.0).abs() < 0.05, "duration {}", v.duration());
        // The decode never upscales the source, keeps the 4:3 aspect, and both sides stay even.
        let (w, h) = v.size();
        assert_eq!((w, h), (160, 120));
        assert!(w <= CAP.0 && h <= CAP.1);
        v.close();
    }

    #[test]
    fn frames_flow_pause_freezes_the_decoder_and_seek_restarts_it() {
        let Some(d) = clip() else { return };
        let mut v = VideoPlayer::default();
        open(&mut v, d.path());

        // 3. Frames arrive, at the size the probe announced.
        assert!(wait_frame(&mut v), "no frame in five seconds");
        let first = v.current().cloned().expect("a frame");
        assert_eq!(first.size(), v.size());

        // 4. Pausing stops the process itself, so nothing new comes out of the pipe.
        v.toggle_pause();
        assert_eq!(v.state(), &State::Paused);
        pump(&mut v, 400);
        let frozen = v.current().cloned().expect("a frame");
        let mut moved = false;
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            v.tick(Instant::now());
            moved |= v.current() != Some(&frozen);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!moved, "a paused decoder still produced a frame");
        v.toggle_pause();
        assert_eq!(v.state(), &State::Playing);
        let until = Instant::now() + Duration::from_secs(5);
        while v.current() == Some(&frozen) && Instant::now() < until {
            v.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(v.current(), Some(&frozen), "frames did not resume");

        // 5. Seeking restarts the decode at the offset asked for, clamped to the file.
        v.toggle_pause();
        assert_eq!(v.state(), &State::Paused);
        v.seek_to(0.2);
        assert!((v.position() - 0.2).abs() < 1e-6, "position {}", v.position());
        v.seek_by(0.5);
        assert!((v.position() - 0.7).abs() < 1e-6, "position {}", v.position());
        v.seek_to(v.duration() + 10.0);
        assert!((v.position() - v.duration()).abs() < 1e-6, "a seek past the end clamps");
        assert_eq!(v.state(), &State::Paused, "a paused seek stays paused");
        // Back to a point with film left, and play on: the frames come from the new offset.
        let before = v.current().cloned();
        v.seek_to(0.1);
        v.toggle_pause();
        assert_eq!(v.state(), &State::Playing);
        let until = Instant::now() + Duration::from_secs(5);
        while v.current() == before.as_ref() && Instant::now() < until {
            v.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(v.current(), before.as_ref(), "no frame after the seek");

        // 6. Dragging the progress track moves the decode to where the pointer is.
        v.toggle_pause();
        assert_eq!(v.state(), &State::Paused);
        let track = Rect::new(10.0, 0.0, 100.0, 18.0);
        v.scrub_press(track, 30.0);
        assert!((v.scrub_preview().unwrap() - 0.2).abs() < 1e-6, "{:?}", v.scrub_preview());
        assert!(v.scrub_drag(70.0));
        assert!((v.scrub_preview().unwrap() - 0.6).abs() < 1e-6, "{:?}", v.scrub_preview());
        assert!(v.scrub_release());
        assert!((v.position() - 0.6).abs() < 1e-6, "position {}", v.position());
        assert!(!v.scrub_release(), "a second release does nothing");
        assert!(v.scrub_preview().is_none());
        v.toggle_pause();

        // The end of the clip leaves the player stopped on the last frame.
        pump(&mut v, 1500);
        assert_eq!(v.state(), &State::Idle, "the stream did not end");
        assert!((v.position() - v.duration()).abs() < 0.01);
        v.close();
    }

    #[test]
    fn closing_leaves_no_decoder_behind() {
        let Some(d) = clip() else { return };
        let mut v = VideoPlayer::default();
        open(&mut v, d.path());
        assert!(wait_frame(&mut v), "no frame in five seconds");
        assert!(!decoders_of(d.path()).is_empty(), "no ffmpeg is running for the clip");
        v.close();
        assert!(!v.is_open());
        assert!(!v.is_running());
        assert_eq!(v.state(), &State::Idle);
        // Killed and reaped: the process is gone, not a zombie waiting for the parent.
        assert_eq!(decoders_of(d.path()), Vec::<u32>::new());
        // Closing twice is harmless, as is the drop at the end of the test.
        v.close();
    }

    #[test]
    fn without_the_tools_nothing_is_spawned() {
        let d = tempfile::tempdir().unwrap();
        let clip = d.path().join("clip.mp4");
        fs::write(&clip, b"not a video").unwrap();
        let mut pipe = FfmpegPipe::with_tools(clip.clone(), clip.clone());
        assert!(pipe.open(&clip, (10, 10), 0.0).is_err());
        assert!(!pipe.is_running());
        assert!(pipe.poll().is_none());
        pipe.close();
        // A player with no usable tools reports it and opens nothing.
        let mut v = VideoPlayer::default();
        assert!(v.open("clip.mp4", &clip, "Note.md", CAP).is_err());
        assert!(!v.is_open());
    }

    #[test]
    fn the_card_becomes_a_player_with_its_controls() {
        let Some(d) = clip() else { return };
        fs::write(d.path().join("Note.md"), "![[clip.mp4]]\n").unwrap();
        let mut app = App::open(d.path()).unwrap();
        app.open_note("Note.md");
        let mut fonts = Fonts::new();
        let mut pm = Pixmap::new(1100, 700).unwrap();

        // The card offers to play the video inside RustSidian, not to hand it to the system.
        let card = render(&mut app, &mut fonts, &mut pm, 1.0);
        let action = "video:open:Note.md|clip.mp4";
        let node = card
            .dom
            .nodes
            .iter()
            .find(|n| n.el.action.as_deref() == Some(action))
            .expect("the card starts the in-app player");
        assert!(node.el.classes.iter().any(|c| c == "media-card-action"), "{:?}", node.el.classes);
        assert_eq!(node.el.text.as_deref(), Some("Reproducir en RustSidian"));

        // Clicking it decodes: the card is replaced by the player once a frame is there.
        app.run(action);
        let mut player = None;
        for _ in 0..250 {
            render(&mut app, &mut fonts, &mut pm, 1.0);
            if app.video.current().is_some() {
                // Freeze the clock so the readout cannot drift while the rest is checked.
                app.video.toggle_pause();
                player = Some(render(&mut app, &mut fonts, &mut pm, 1.0));
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let frame = player.expect("the player never drew a frame");
        assert_eq!(app.video.state(), &State::Paused);

        let has = |n: &rustsidian_ui::dom::Node, c: &str| n.el.classes.iter().any(|x| x == c);
        assert!(frame.dom.nodes.iter().any(|n| has(n, "video-player")), "no player widget");
        let frame_node =
            frame.dom.nodes.iter().position(|n| has(n, "video-player-frame")).expect("no frame box");
        // The decoded pixels are what the frame box paints.
        let rect = frame.dom.nodes[frame_node].rect;
        let scene = frame.dom.paint();
        assert!(
            scene.cmds.iter().any(|c| matches!(c, Cmd::Image { rect: r, .. } if *r == rect)),
            "the frame box draws no image"
        );

        let actions: Vec<&str> = frame.dom.nodes.iter().filter_map(|n| n.el.action.as_deref()).collect();
        for want in ["video:toggle", "video:scrub", "video:speed", "video:fullscreen", "video:close"] {
            assert!(actions.contains(&want), "{want} is missing from {actions:?}");
        }
        // The ±10 s buttons were two controls doing what the bar and the arrows already do.
        assert!(
            !actions.iter().any(|a| a.starts_with("video:seek:")),
            "the seek buttons are gone: {actions:?}"
        );
        // The speed control says what it does instead of being an unreadable glyph.
        let speed = frame
            .dom
            .nodes
            .iter()
            .find(|n| n.el.action.as_deref() == Some("video:speed"))
            .and_then(|n| n.el.text.clone())
            .expect("the speed control has no label");
        assert_eq!(speed, "1x");
        // Everything is laid out and visible: a zero-width track could not be dragged.
        let rect_of =
            |class: &str| frame.dom.nodes.iter().find(|n| has(n, class)).map(|n| n.rect).unwrap_or_default();
        for class in [
            "video-player",
            "video-player-frame",
            "video-player-controls",
            "video-player-time",
            "video-player-thumb",
            "video-player-btn",
        ] {
            let r = rect_of(class);
            assert!(r.w > 0.0 && r.h > 0.0, ".{class} is {r:?}");
        }
        let track = rect_of("video-player-track");
        assert!(track.w > 100.0, "the scrub track is {track:?}");
        // The whole widget stays inside the column it is embedded in.
        assert!(rect_of("video-player").w <= 640.0);
        // Close and the seek buttons carry a label for the screen reader.
        let labelled = |label: &str| {
            frame.dom.nodes.iter().any(|n| {
                n.el.attrs.iter().any(|(k, v)| k == "aria-label" && v == label)
                    && n.el.action.as_deref() == Some("video:close")
            })
        };
        assert!(labelled("Cerrar el reproductor"));
        let time = frame
            .dom
            .nodes
            .iter()
            .find(|n| has(n, "video-player-time"))
            .and_then(|n| n.el.text.clone())
            .expect("no time readout");
        // `0:00 / 0:01`; the leading number depends on how long the first frame took.
        assert!(time.starts_with("0:0") && time.ends_with(" / 0:01"), "{time}");

        // Closing kills the decoder and the card comes back.
        app.run("video:close");
        assert!(!app.video.is_open());
        let card = render(&mut app, &mut fonts, &mut pm, 1.0);
        assert!(card.dom.nodes.iter().any(|n| n.el.action.as_deref() == Some(action)));
        assert!(!card.dom.nodes.iter().any(|n| has(n, "video-player")));
        assert_eq!(decoders_of(d.path()), Vec::<u32>::new());
    }

    #[test]
    fn the_speed_control_cycles_the_rates_and_the_readout_follows() {
        let mut v = VideoPlayer::default();
        // Nothing set yet: the control says what it does, and what it does is normal speed.
        assert_eq!(v.speed_label(), "1x");
        let mut seen = Vec::new();
        for _ in 0..5 {
            v.cycle_speed();
            seen.push(v.speed_label());
        }
        assert_eq!(seen, ["1.25x", "1.5x", "2x", "0.75x", "1x"]);
        assert!(!v.is_open(), "changing the speed opens nothing on its own");
    }

    #[test]
    fn the_fullscreen_button_shows_the_player_over_the_window() {
        let Some(d) = clip() else { return };
        fs::write(d.path().join("Note.md"), "![[clip.mp4]]\n").unwrap();
        let mut app = App::open(d.path()).unwrap();
        app.open_note("Note.md");
        let mut fonts = Fonts::new();
        let mut pm = Pixmap::new(1100, 700).unwrap();
        app.run("video:open:Note.md|clip.mp4");
        for _ in 0..250 {
            render(&mut app, &mut fonts, &mut pm, 1.0);
            if app.video.current().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        app.video.toggle_pause();
        let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
        let button = frame
            .dom
            .nodes
            .iter()
            .position(|n| n.el.action.as_deref() == Some("video:fullscreen"))
            .expect("the player offers full screen");
        let r = frame.dom.nodes[button].rect;
        // It is inside the card, not clipped away by a narrow panel.
        assert!(r.w > 0.0 && r.h > 0.0, "{r:?}");
        let card = frame
            .dom
            .nodes
            .iter()
            .find(|n| n.el.classes.iter().any(|c| c == "video-player"))
            .map(|n| n.rect)
            .expect("no player");
        assert!(r.x >= card.x && r.x + r.w <= card.x + card.w, "the button is cut off: {r:?} in {card:?}");

        // The click opens the modal; the note keeps no player of its own meanwhile.
        app.click(&frame, &mut fonts, r.x + 2.0, r.y + 2.0, false);
        assert!(app.video_fullscreen, "the button did not open the full-screen view");
        let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
        let has = |c: &str| frame.dom.nodes.iter().any(|n| n.el.classes.iter().any(|x| x == c));
        assert!(has("mod-video"), "the video has no modal of its own");
        assert!(has("video-fullscreen"), "no backdrop behind it");
        // The card in the note is gone while the player is over the window.
        let cards = frame.dom.nodes.iter().filter(|n| n.el.classes.iter().any(|c| c == "video-player")).count();
        assert_eq!(cards, 1, "the player is drawn once");

        // Escape goes back to the player in its note.
        app.key(rustsidian_app::Key::parse("Escape").unwrap());
        assert!(!app.video_fullscreen);
        let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
        assert!(!frame.dom.nodes.iter().any(|n| n.el.classes.iter().any(|x| x == "mod-video")));
        assert!(frame.dom.nodes.iter().any(|n| n.el.classes.iter().any(|x| x == "video-player")));
        app.run("video:close");
    }

    /// How long the clip takes to play out from the start, in milliseconds.
    fn play_out(v: &mut VideoPlayer) -> u128 {
        let t0 = Instant::now();
        v.seek_to(0.0);
        while t0.elapsed() < Duration::from_secs(10) {
            v.tick(Instant::now());
            if v.state() == &State::Idle {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        t0.elapsed().as_millis()
    }

    #[test]
    fn a_higher_speed_plays_the_clip_out_sooner() {
        let Some(d) = clip() else { return };
        let mut normal = VideoPlayer::default();
        open(&mut normal, d.path());
        assert!(wait_frame(&mut normal));
        let at_one = play_out(&mut normal);
        assert_eq!(normal.speed(), 1.0);

        let mut fast = VideoPlayer::default();
        open(&mut fast, d.path());
        assert!(wait_frame(&mut fast));
        fast.set_speed(2.0);
        assert_eq!(fast.speed_label(), "2x");
        let at_two = play_out(&mut fast);
        normal.close();
        fast.close();
        assert!(
            at_two * 100 < at_one * 80,
            "2x took {at_two} ms against {at_one} ms at 1x"
        );
    }
}
