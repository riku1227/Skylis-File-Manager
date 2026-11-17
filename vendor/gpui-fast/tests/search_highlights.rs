use gpui::{TextRun, TruncateFrom, font, px};

#[gpui::test]
fn end_ellipsis_does_not_inherit_match_background(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let name: gpui::SharedString = "report_report_report_report_report.txt".into();
        let run = TextRun {
            len: name.len(),
            font: font("test"),
            color: gpui::black(),
            background_color: Some(gpui::yellow()),
            underline: None,
            strikethrough: None,
        };
        let mut wrapper = cx.text_system().line_wrapper(font("test"), px(14.));
        let source = [run];
        let (text, runs) =
            wrapper.truncate_line(name.clone(), px(90.), "…", &source, TruncateFrom::End);
        assert!(text.ends_with('…'));
        assert!(text.len() < name.len());
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), text.len());
        assert!(runs.iter().any(|r| r.background_color.is_some()));
        assert!(
            runs.last().unwrap().background_color.is_none(),
            "ellipsis falsely marked as a match"
        );
    });
}

#[gpui::test]
fn hidden_and_partly_visible_matches_do_not_color_ellipsis(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let name: gpui::SharedString = "abcdefghijklmnopqrstuvwxyz.txt".into();
        let plain = TextRun {
            len: name.len(),
            font: font("test"),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let mut wrapper = cx.text_system().line_wrapper(font("test"), px(14.));
        let base = [plain.clone()];
        let (cut, _) = wrapper.truncate_line(name.clone(), px(90.), "…", &base, TruncateFrom::End);
        let kept = cut.len() - '…'.len_utf8();
        assert!(kept > 2);
        for start in [kept, kept - 2] {
            let mut marked = plain.clone();
            marked.len = 3;
            marked.background_color = Some(gpui::yellow());
            let runs = [
                TextRun {
                    len: start,
                    ..plain.clone()
                },
                marked,
                TextRun {
                    len: name.len() - start - 3,
                    ..plain.clone()
                },
            ];
            let (text, runs) =
                wrapper.truncate_line(name.clone(), px(90.), "…", &runs, TruncateFrom::End);
            assert_eq!(text, cut);
            assert!(runs.last().unwrap().background_color.is_none());
            assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), text.len());
            assert_eq!(
                runs.iter().any(|r| r.background_color.is_some()),
                start < kept
            );
        }
    });
}
