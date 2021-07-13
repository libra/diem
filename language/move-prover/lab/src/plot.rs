// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Functions for reading benchmark data and converting them to graphics.
// those results and plotting them.

use crate::{
    benchmark,
    benchmark::{Benchmark, BenchmarkData},
};
use anyhow::Context;
use clap::{App, Arg};
use itertools::Itertools;
use plotters::{
    coord::types::RangedCoordu32,
    prelude::{
        Cartesian2d, IntoDrawingArea, IntoFont, RGBColor, Rectangle, SVGBackend, ShapeStyle, Text,
        BLACK, BLUE, CYAN, GREEN, MAGENTA, RED, WHITE, YELLOW,
    },
};
use std::collections::BTreeMap;

// =====================================================================================
// Command line interface

pub fn plot_svg(args: &[String]) -> anyhow::Result<()> {
    let is_number = |s: String| {
        s.parse::<usize>()
            .map(|_| ())
            .map_err(|_| "expected number".to_string())
    };
    let cmd_line_parser = App::new("plot")
        .version("0.1.0")
        .about("Benchmark plotter for the Move Prover")
        .author("The Diem Core Contributors")
        .arg(
            Arg::with_name("out")
                .long("out")
                .takes_value(true)
                .value_name("FILE")
                .help("file where output will be written too"),
        )
        .arg(
            Arg::with_name("sort")
                .long("sort")
                .help("whether to sort the benchmark data based on the first data file"),
        )
        .arg(
            Arg::with_name("top")
                .long("top")
                .takes_value(true)
                .value_name("NUMBER")
                .validator(is_number)
                .help("plot only the top N entries"),
        )
        .arg(
            Arg::with_name("data-files")
                .multiple(true)
                .value_name("PATH_TO_BENCHMARK_DATA")
                .min_values(1)
                .default_value("")
                .empty_values(false)
                .help("the benchmark data files to plot"),
        );
    let matches = cmd_line_parser.get_matches_from_safe(args)?;
    let get_vec = |s: &str| -> Vec<String> {
        match matches.values_of(s) {
            Some(vs) => vs.map(|v| v.to_string()).collect(),
            _ => vec![],
        }
    };
    let out_file = if matches.is_present("out") {
        matches.value_of("out").unwrap().to_string()
    } else {
        "plot.svg".to_owned()
    };
    let sort = matches.is_present("sort");
    let top = if matches.is_present("top") {
        Some(matches.value_of("top").unwrap().parse::<usize>()?)
    } else {
        None
    };
    let data_files = get_vec("data-files");
    let mut data = vec![];
    for file in data_files {
        data.push(benchmark::read_benchmark(&file).context(format!("cannot open `{}`", file))?);
    }

    if sort {
        data[0].sort();
    }

    if let Some(n) = top {
        data[0].take(n)
    }

    println!("plotting to `{}`", out_file);
    plot_benchmarks_to_file(&out_file, data.as_slice())
}

pub const LIGHT_GRAY: RGBColor = RGBColor(0xb4, 0xb4, 0xb4);
pub const MEDIUM_GRAY: RGBColor = RGBColor(0x90, 0x90, 0x90);
pub const GRAY: RGBColor = RGBColor(0x63, 0x63, 0x63);
pub const DARK_GRAY: RGBColor = RGBColor(0x49, 0x48, 0x48);

pub const GRAY_PALETTE: &[&RGBColor] = &[&LIGHT_GRAY, &MEDIUM_GRAY, &GRAY, &DARK_GRAY, &BLACK];
pub const COLOR_PALETTE: &[&RGBColor] = &[&GREEN, &BLUE, &RED, &CYAN, &YELLOW, &MAGENTA];

/// Plot a set of benchmarks to an SVG file.
/// The first entry in the list determines ranking: only data points for labels used in this
/// benchmark are plotted, and in the order they appear in the first benchmark.
pub fn plot_benchmarks_to_file(fname: &str, benchmarks: &[Benchmark]) -> anyhow::Result<()> {
    #[derive(Clone, Copy)]
    enum Result {
        Duration(usize),
        Error(usize),
        Timeout,
    }
    // Join matching samples over all benchmarks. This maps from sample name
    // to a pair of configuration and duration, or whether its a timeout or an error.
    let mut joined: BTreeMap<&str, Vec<(&str, Result)>> = BTreeMap::new();
    for Benchmark { config, data } in benchmarks {
        for BenchmarkData {
            name,
            duration,
            status,
        } in data
        {
            match status.as_str() {
                "ok" => joined
                    .entry(name)
                    .or_default()
                    .push((config, Result::Duration(*duration))),
                "timeout" => joined
                    .entry(name)
                    .or_default()
                    .push((config, Result::Timeout)),
                _ => joined
                    .entry(name)
                    .or_default()
                    .push((config, Result::Error(*duration))),
            }
        }
    }

    // Rearrange samples in order of first benchmark.
    let joined = benchmarks[0]
        .data
        .iter()
        .map(|d| (d.name.as_str(), joined.get(d.name.as_str()).unwrap()))
        .collect_vec();

    // Compute maximal duration and data points, for correct scaling.
    let data_points = joined.len() as u32;
    let max_duration = joined
        .iter()
        .map(|(_, e)| e.iter().map(|(_, d)| *d))
        .flatten()
        .filter_map(|r| {
            if let Result::Duration(d) | Result::Error(d) = r {
                Some(d)
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0) as u32;

    // We are drawing data points as horizontal bars, therefore x-axis is max_duration
    // and y-axis datapoints.
    let real_x = 1000u32;
    let real_y = data_points * 60u32;
    let root = SVGBackend::new(fname, (real_x, real_y)).into_drawing_area();

    let duration_percent = |p: usize| ((max_duration as f64) * (p as f64) / 100f64) as u32;
    let bar = |y: u32, w: u32, style| Rectangle::new([(0, y + 1), (w, y + 8)], style);
    let label = |s: &str, x: u32, y: u32, h| {
        Text::new(s.to_string(), (x, y), ("sans-serif", h).into_font())
    };
    let filled_shape = |i: usize| ShapeStyle::from(GRAY_PALETTE[i]).filled();
    let stroke_shape =
        |i: usize| ShapeStyle::from(GRAY_PALETTE[i]).stroke_width(duration_percent(1));

    let root = root.apply_coord_spec(Cartesian2d::<RangedCoordu32, RangedCoordu32>::new(
        0..max_duration + duration_percent(10), // + 10% for label
        0..(data_points + 1) * ((1 + benchmarks.len() as u32) * 10),
        (0..real_x as i32, 0..real_y as i32),
    ));
    root.fill(&WHITE)?;
    let mut ycoord = 0;

    // Draw legend
    for (i, benchmark) in benchmarks.iter().enumerate() {
        root.draw(&bar(ycoord, duration_percent(5), filled_shape(i)))?;
        root.draw(&label(
            &format!("= {}", benchmark.config),
            duration_percent(6),
            ycoord + 2,
            15.0,
        ))?;
        ycoord += 10;
    }
    ycoord += 10;
    // Draw samples.
    for (sample, variants) in joined {
        root.draw(&label(sample, 0, ycoord, 15.0))?;
        ycoord += 7;
        for (i, (_, result)) in variants.iter().enumerate() {
            let (weight, note, style) = match result {
                Result::Duration(d) => (
                    *d as u32,
                    format!("{:.3}", (*d as f64) / 1000f64),
                    filled_shape(i),
                ),
                Result::Timeout => (max_duration, "timeout".to_string(), stroke_shape(i)),
                Result::Error(d) => (*d as u32, "error".to_string(), filled_shape(i)),
            };
            root.draw(&bar(ycoord, weight, style))?;
            root.draw(&label(
                &note,
                weight + duration_percent(1),
                ycoord + 2,
                13.0,
            ))?;
            ycoord += 10;
        }
        ycoord += 3;
    }
    Ok(())
}
