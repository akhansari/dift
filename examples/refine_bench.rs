//! Algorithm quality gates and five warmed timing samples, without syntax or layout.
//! Includes the internal engine to benchmark production code without expanding its API.
use config::Emphasis;
use dift::{
    AppResult, config, model,
    render::{Cancellation, Cancelled},
};
#[path = "../src/render/refine.rs"]
mod refine;
use model::{LineKind, SourceLine};
use refine::{ALGORITHMS, Algorithms};
use similar::{Algorithm, TextDiff, capture_diff_slices_by_key_deadline};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const CANDIDATES: [Algorithm; 5] = [
    Algorithm::Myers,
    Algorithm::RawMyers,
    Algorithm::Patience,
    Algorithm::Histogram,
    Algorithm::Hunt,
];
const REPETITIONS: usize = 20;
type Coordinates = Vec<(Option<usize>, Option<usize>)>;

struct Block {
    name: &'static str,
    lines: Vec<SourceLine>,
    old: Vec<usize>,
    new: Vec<usize>,
    expected: Coordinates,
}

impl Block {
    fn new(name: &'static str, old: &[&str], new: &[&str], expected: Coordinates) -> Self {
        Self {
            name,
            lines: old
                .iter()
                .enumerate()
                .map(|(i, text)| SourceLine {
                    kind: LineKind::Removed,
                    old: Some(i + 1),
                    new: None,
                    text: (*text).into(),
                    no_newline: false,
                })
                .chain(new.iter().enumerate().map(|(i, text)| SourceLine {
                    kind: LineKind::Added,
                    old: None,
                    new: Some(i + 1),
                    text: (*text).into(),
                    no_newline: false,
                }))
                .collect(),
            old: (0..old.len()).collect(),
            new: (old.len()..old.len() + new.len()).collect(),
            expected,
        }
    }

    fn matches(&self, algorithms: Algorithms) -> AppResult<bool> {
        let pairs = refine::align(
            &self.lines,
            &self.old,
            &self.new,
            algorithms,
            Instant::now() + Duration::from_secs(1),
            &Cancellation::default(),
        )?;
        let actual: Coordinates = pairs
            .iter()
            .map(|p| {
                (
                    p.old.and_then(|i| self.lines[i].old),
                    p.new.and_then(|i| self.lines[i].new),
                )
            })
            .collect();
        if actual != self.expected {
            eprintln!(
                "Rejected {}: expected {:?}, got {:?}",
                self.name, self.expected, actual
            );
        }
        Ok(actual == self.expected)
    }
}

fn blocks() -> Vec<Block> {
    vec![
        Block::new(
            "insertions",
            &["let answer = 1;", "return answer;"],
            &[
                "// explain",
                "validate();",
                "let answer = 2;",
                "return answer;",
            ],
            vec![
                (None, Some(1)),
                (None, Some(2)),
                (Some(1), Some(3)),
                (Some(2), Some(4)),
            ],
        ),
        Block::new(
            "deletions",
            &[
                "// explain",
                "validate();",
                "let answer = 1;",
                "return answer;",
            ],
            &["let answer = 2;", "return answer;"],
            vec![
                (Some(1), None),
                (Some(2), None),
                (Some(3), Some(1)),
                (Some(4), Some(2)),
            ],
        ),
        Block::new(
            "repeated",
            &["start();", "}", "finish();", "}"],
            &["start();", "}", "added();", "}", "finish();", "}"],
            vec![
                (Some(1), Some(1)),
                (Some(2), Some(2)),
                (None, Some(3)),
                (None, Some(4)),
                (Some(3), Some(5)),
                (Some(4), Some(6)),
            ],
        ),
        Block::new(
            "indentation",
            &["run();", "}"],
            &["  run();", "  }"],
            vec![(Some(1), Some(1)), (Some(2), Some(2))],
        ),
        Block::new(
            "ties",
            &["let item = 1;", "let item = 2;"],
            &["let item = 3;"],
            vec![(Some(1), Some(1)), (Some(2), None)],
        ),
        Block::new(
            "unrelated",
            &["alpha"],
            &["ΩΩΩ"],
            vec![(Some(1), None), (None, Some(1))],
        ),
        Block::new("empty", &[], &[""], vec![(None, Some(1))]),
    ]
}

struct InlineFixture {
    name: &'static str,
    old: &'static str,
    new: &'static str,
    left: &'static [&'static str],
    right: &'static [&'static str],
}

const WORDS: &[InlineFixture] = &[
    InlineFixture {
        name: "identifier",
        old: "const result = user_count;",
        new: "const result = user_total;",
        left: &["user_count"],
        right: &["user_total"],
    },
    InlineFixture {
        name: "semantic",
        old: "The came.",
        new: "The cat came.",
        left: &[],
        right: &["cat "],
    },
    InlineFixture {
        name: "unicode",
        old: "let café = \"été\";",
        new: "let café = \"hiver\";",
        left: &["été"],
        right: &["hiver"],
    },
    InlineFixture {
        name: "whitespace",
        old: "run();",
        new: "  run();  ",
        left: &[],
        right: &["  ", "  "],
    },
    InlineFixture {
        name: "punctuation",
        old: "f(x) y",
        new: "f(z) y",
        left: &["x"],
        right: &["z"],
    },
    InlineFixture {
        name: "disjoint",
        old: "alpha",
        new: "ΩΩΩ",
        left: &[],
        right: &[],
    },
    InlineFixture {
        name: "empty",
        old: "",
        new: "",
        left: &[],
        right: &[],
    },
];

const GRAPHEMES: &[InlineFixture] = &[
    InlineFixture {
        name: "digit",
        old: "const count = 1000;",
        new: "const count = 1001;",
        left: &["0"],
        right: &["1"],
    },
    InlineFixture {
        name: "identifier suffix",
        old: "const result = user_count;",
        new: "const result = user_counts;",
        left: &[],
        right: &["s"],
    },
    InlineFixture {
        name: "combining mark",
        old: "let cafe\u{301} = 1;",
        new: "let cafe\u{300} = 1;",
        left: &["e\u{301}"],
        right: &["e\u{300}"],
    },
    InlineFixture {
        name: "emoji sequence",
        old: "let icon = \"👩‍💻\";",
        new: "let icon = \"👨‍💻\";",
        left: &["👩‍💻"],
        right: &["👨‍💻"],
    },
    InlineFixture {
        name: "semantic phrase",
        old: "The came.",
        new: "The cat came.",
        left: &[],
        right: &["cat "],
    },
    InlineFixture {
        name: "whitespace",
        old: "run();",
        new: "  run();  ",
        left: &[],
        right: &["  ", "  "],
    },
];

fn matches_inline(
    fixture: &InlineFixture,
    algorithm: Algorithm,
    mode: Emphasis,
) -> AppResult<bool> {
    let (left, right) = refine::emphasize(
        fixture.old,
        fixture.new,
        algorithm,
        mode,
        Instant::now() + Duration::from_secs(1),
        &Cancellation::default(),
    )?;
    let left: Vec<_> = left.into_iter().map(|r| &fixture.old[r]).collect();
    let right: Vec<_> = right.into_iter().map(|r| &fixture.new[r]).collect();
    let matches = if mode == Emphasis::Off {
        left.is_empty() && right.is_empty()
    } else {
        left == fixture.left && right == fixture.right
    };
    if !matches {
        eprintln!("Rejected {mode:?} {}: {left:?} / {right:?}", fixture.name);
    }
    Ok(matches)
}

#[derive(Clone, Copy, Debug)]
enum Role {
    Block,
    Similarity,
    Inline,
}

impl Role {
    fn algorithms(self, candidate: Algorithm) -> Algorithms {
        match self {
            Self::Block => Algorithms {
                block: candidate,
                ..ALGORITHMS
            },
            Self::Similarity => Algorithms {
                similarity: candidate,
                ..ALGORITHMS
            },
            Self::Inline => Algorithms {
                inline: candidate,
                ..ALGORITHMS
            },
        }
    }
    fn preferred(self) -> Algorithm {
        match self {
            Self::Block => Algorithm::Histogram,
            Self::Similarity => Algorithm::Myers,
            Self::Inline => Algorithm::Patience,
        }
    }
}

fn eligible(role: Role, candidate: Algorithm, blocks: &[Block]) -> AppResult<bool> {
    let algorithms = role.algorithms(candidate);
    match role {
        Role::Block | Role::Similarity => {
            for block in blocks {
                if !block.matches(algorithms)? {
                    return Ok(false);
                }
            }
        }
        Role::Inline => {
            for words in WORDS {
                if !matches_inline(words, algorithms.inline, Emphasis::Word)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn median(mut operation: impl FnMut() -> AppResult<()>) -> AppResult<f64> {
    // Warm each fixture before recording five equal-size batches.
    for _ in 0..REPETITIONS {
        operation()?;
    }
    let mut samples = [0.0_f64; 5];
    for sample in &mut samples {
        let start = Instant::now();
        for _ in 0..REPETITIONS {
            operation()?;
        }
        *sample = start.elapsed().as_secs_f64() * 1_000_000.0 / REPETITIONS as f64;
    }
    samples.sort_by(f64::total_cmp);
    Ok(samples[2])
}

fn compare<T: Copy, const N: usize>(
    candidates: [T; N],
    mut operation: impl FnMut(T) -> AppResult<()>,
) -> AppResult<[f64; N]> {
    for candidate in candidates {
        for _ in 0..REPETITIONS {
            operation(candidate)?;
        }
    }
    let mut samples = [[0.0_f64; N]; 5];
    // Rotate candidates to distribute the effect of sampling order.
    for (round, samples) in samples.iter_mut().enumerate() {
        for offset in 0..N {
            let index = (round + offset) % N;
            let start = Instant::now();
            for _ in 0..REPETITIONS {
                operation(candidates[index])?;
            }
            samples[index] = start.elapsed().as_secs_f64() * 1_000_000.0 / REPETITIONS as f64;
        }
    }
    Ok(std::array::from_fn(|index| {
        let mut times = samples.map(|round| round[index]);
        times.sort_by(f64::total_cmp);
        times[2]
    }))
}

fn measure(role: Role, blocks: &[Block]) -> AppResult<[f64; 5]> {
    let mut total = [0.0; 5];
    match role {
        Role::Block | Role::Similarity => {
            for block in blocks {
                let times = compare(CANDIDATES, |algorithm| {
                    let deadline = Instant::now() + Duration::from_secs(1);
                    match role {
                        Role::Block => {
                            black_box(capture_diff_slices_by_key_deadline(
                                algorithm,
                                &block.old,
                                &block.new,
                                |i| block.lines[*i].text.trim(),
                                Some(deadline),
                            ));
                        }
                        Role::Similarity => {
                            for a in &block.old {
                                for b in &block.new {
                                    black_box(
                                        TextDiff::configure()
                                            .algorithm(algorithm)
                                            .deadline(deadline)
                                            .diff_chars(
                                                block.lines[*a].text.trim(),
                                                block.lines[*b].text.trim(),
                                            )
                                            .ratio(),
                                    );
                                }
                            }
                        }
                        Role::Inline => {}
                    }
                    Ok(())
                })?;
                for (total, time) in total.iter_mut().zip(times) {
                    *total += time;
                }
            }
        }
        Role::Inline => {
            for words in WORDS {
                let times = compare(CANDIDATES, |algorithm| {
                    black_box(refine::emphasize(
                        words.old,
                        words.new,
                        algorithm,
                        Emphasis::Word,
                        Instant::now() + Duration::from_secs(1),
                        &Cancellation::default(),
                    )?);
                    Ok(())
                })?;
                for (total, time) in total.iter_mut().zip(times) {
                    *total += time;
                }
            }
        }
    }
    Ok(total)
}

fn workloads(mut blocks: Vec<Block>) -> Vec<Block> {
    let old: Vec<_> = (0..32)
        .map(|i| format!("export const item_{i} = {{ count: {i} }};"))
        .collect();
    let dense: Vec<_> = (0..32)
        .map(|i| format!("export const item_{i} = {{ count: {} }};", i + 1))
        .collect();
    let sparse: Vec<_> = old
        .iter()
        .take(31)
        .map(String::as_str)
        .chain(["export const item_31 = { count: 999 };"])
        .collect();
    let old: Vec<_> = old.iter().map(String::as_str).collect();
    let dense: Vec<_> = dense.iter().map(String::as_str).collect();
    blocks.push(Block::new("sparse source", &old, &sparse, Vec::new()));
    blocks.push(Block::new("dense source", &old, &dense, Vec::new()));
    blocks.push(Block::new(
        "repetitive",
        &["}"; 32],
        &["}", "}", "new", "}", "}"],
        Vec::new(),
    ));
    blocks
}

fn compare_emphasis_modes() -> AppResult<()> {
    let modes = [Emphasis::Word, Emphasis::Grapheme, Emphasis::Off];
    for mode in modes {
        let fixtures = if mode == Emphasis::Grapheme {
            GRAPHEMES
        } else {
            WORDS
        };
        for fixture in fixtures {
            if !matches_inline(fixture, ALGORITHMS.inline, mode)? {
                return Err(format!("{mode:?} failed its emphasis fixtures").into());
            }
        }
    }
    println!("emphasis_fixture,word_us,grapheme_us,off_us");
    // Compare each mode over identical inputs using the fixed production algorithm.
    for fixture in WORDS.iter().chain(GRAPHEMES) {
        let times = compare(modes, |mode| {
            black_box(refine::emphasize(
                fixture.old,
                fixture.new,
                ALGORITHMS.inline,
                mode,
                Instant::now() + Duration::from_secs(1),
                &Cancellation::default(),
            )?);
            Ok(())
        })?;
        println!(
            "{},{:.3},{:.3},{:.3}",
            fixture.name, times[0], times[1], times[2]
        );
    }
    Ok(())
}

fn main() -> AppResult<()> {
    let fixtures = blocks();
    let workloads = workloads(blocks());
    println!("role,algorithm,eligible,aggregate_median_us");
    for role in [Role::Block, Role::Similarity, Role::Inline] {
        let mut scores = Vec::new();
        let times = measure(role, &workloads)?;
        for (candidate, time) in CANDIDATES.into_iter().zip(times) {
            let eligible = eligible(role, candidate, &fixtures)?;
            println!("{role:?},{candidate:?},{eligible},{time:.3}");
            if eligible {
                scores.push((candidate, time));
            }
        }
        scores.sort_by(|a, b| a.1.total_cmp(&b.1));
        let fastest = scores
            .first()
            .ok_or("No algorithm passed the behavioral fixtures")?;
        let selected = scores
            .iter()
            .find(|(alg, time)| *alg == role.preferred() && *time <= fastest.1 * 1.1)
            .unwrap_or(fastest);
        println!("selected {role:?}: {:?} ({:.3} us)", selected.0, selected.1);
    }
    // Full alignment includes mapping and similarity, while the table isolates each role.
    for block in &workloads {
        let time = median(|| {
            black_box(refine::align(
                &block.lines,
                &block.old,
                &block.new,
                ALGORITHMS,
                Instant::now() + Duration::from_millis(3),
                &Cancellation::default(),
            )?);
            Ok(())
        })?;
        println!("alignment {}: {time:.3} us", block.name);
    }
    compare_emphasis_modes()
}
