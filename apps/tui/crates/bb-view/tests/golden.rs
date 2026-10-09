use bb_view::{TimelineTextOptions, format_thread_timeline_text};
use bb_wire::timeline::TimelineRow;

const FIXTURE: &str = include_str!("fixtures/rich_timeline.json");
const EXPECTED: &str = include_str!("fixtures/rich_timeline.txt");
const EXPECTED_VERBOSE: &str = include_str!("fixtures/rich_timeline.verbose.txt");
const EXPECTED_COLOR: &str = include_str!("fixtures/rich_timeline.color.txt");

fn rows() -> Vec<TimelineRow> {
    let rows: Vec<TimelineRow> = serde_json::from_str(FIXTURE).expect("fixture rows");
    assert!(
        rows.iter().all(|row| !row.is_unknown()),
        "fixture rows must parse as known kinds"
    );
    rows
}

fn assert_text(actual: String, expected: &str) {
    let actual = format!("{actual}\n");
    if actual != expected {
        let first_difference = actual
            .lines()
            .zip(expected.lines())
            .enumerate()
            .find(|(_, (left, right))| left != right);
        panic!(
            "golden mismatch at {first_difference:?}\n--- actual ---\n{actual}\n--- expected ---\n{expected}"
        );
    }
}

#[test]
#[ignore = "set BB_VIEW_LIVE_DIR to a directory of captured /timeline responses"]
fn formats_captured_live_timelines() {
    let dir = std::env::var("BB_VIEW_LIVE_DIR").expect("BB_VIEW_LIVE_DIR");
    for entry in std::fs::read_dir(&dir).expect("live dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("read timeline");
        let response: bb_wire::timeline::ThreadTimelineResponse =
            serde_json::from_str(&body).expect("timeline response");
        for (mode, verbose) in [("default", false), ("verbose", true)] {
            let text = format_thread_timeline_text(
                &response.rows,
                TimelineTextOptions {
                    verbose,
                    color: false,
                },
            );
            let output = format!("{}.{mode}.rs.txt", path.display());
            std::fs::write(output, text).expect("write output");
        }
    }
}

#[test]
fn matches_format_thread_timeline_text_output() {
    assert_text(
        format_thread_timeline_text(&rows(), TimelineTextOptions::default()),
        EXPECTED,
    );
}

#[test]
fn matches_verbose_output() {
    assert_text(
        format_thread_timeline_text(
            &rows(),
            TimelineTextOptions {
                verbose: true,
                color: false,
            },
        ),
        EXPECTED_VERBOSE,
    );
}

#[test]
fn matches_colored_output() {
    assert_text(
        format_thread_timeline_text(
            &rows(),
            TimelineTextOptions {
                verbose: false,
                color: true,
            },
        ),
        EXPECTED_COLOR,
    );
}
