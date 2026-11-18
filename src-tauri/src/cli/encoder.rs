use std::{
    process::exit,
    sync::{Arc, atomic::AtomicBool},
};

use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::error;
use tokio::sync::Mutex;

use crate::{
    cli::{
        args::Args,
        utils::{create_tasks, read_config},
    },
    utils::errors::ProcessError,
    worker::work,
};

pub async fn run() -> Result<(), ProcessError> {
    super::resources::initialize()?;
    let args = Args::parse();
    let config = read_config(&args)?;
    let args = args.init(&config).await?;

    if args.files.is_empty() {
        error!("Add files to encode: -f <[FILES]>");
        exit(1);
    }

    // let preset_length = args.presets.as_ref().unwrap_or(&vec![]).len();
    let tasks = create_tasks(&config, args).await;
    let task_length = tasks.len();

    let multi_prog = MultiProgress::new();
    let sty = ProgressStyle::with_template(
        "{prefix}: [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
    )
    .unwrap()
    .progress_chars("#-");

    let n = 100;
    let current = multi_prog.add(ProgressBar::new(n));
    current.set_style(sty.clone());
    let all = multi_prog.add(ProgressBar::new(n));
    all.set_style(sty.clone());

    // let length = tasks.len() * preset_length;

    for (i, task) in tasks.iter().enumerate() {
        work(
            None,
            config.clone(),
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicBool::new(true)),
            task.clone(),
            Some(current.clone()),
        )
        .await?;

        let prog = ((i + 1) * 100 / task_length) as u64;

        current.set_prefix("Current");
        current.finish_with_message("done...");
        all.set_prefix("OverAll");
        all.set_position(prog);
    }

    all.finish_with_message("all jobs done");
    println!("\n");
    multi_prog.clear().unwrap();

    Ok(())
}
