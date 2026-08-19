//! Shared interactive picker for stored prompts.

use std::{borrow::Cow, sync::Arc};

use anyhow::Result;
use skim::prelude::*;

const HEIGHT: &str = "10";

pub(crate) fn pick(prompts: &[String], colors: [Option<u8>; 6]) -> Result<Option<String>> {
    let (sender, receiver): (SkimItemSender, SkimItemReceiver) = unbounded();
    let items = prompts
        .iter()
        .rev()
        .map(|prompt| Arc::new(PromptItem::new(prompt.clone())) as Arc<dyn SkimItem>)
        .collect();
    sender.send(items)?;
    drop(sender);

    let names = [
        "prompt",
        "query",
        "fg",
        "matched",
        "current",
        "current_match",
    ];
    let colors = names
        .into_iter()
        .zip(colors)
        .filter_map(|(name, color)| color.map(|color| format!("{name}:{color}")))
        .collect::<Vec<_>>();
    let mut options = SkimOptionsBuilder::default();
    options.height(HEIGHT).prompt("history> ");
    if !colors.is_empty() {
        options.color(colors.join(","));
    }
    let options = options.multi(false).build()?;
    let output = Skim::run_with(options, Some(receiver))
        .map_err(|error| anyhow::anyhow!("run history picker: {error}"))?;
    Ok((!output.is_abort)
        .then(|| output.selected_items.into_iter().next())
        .flatten()
        .map(|item| item.output().into_owned()))
}

struct PromptItem {
    value: String,
    display: String,
}

impl PromptItem {
    fn new(value: String) -> Self {
        Self {
            display: value.replace('\n', " ↵ "),
            value,
        }
    }
}

impl SkimItem for PromptItem {
    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.display)
    }

    fn output(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.value)
    }
}

#[cfg(test)]
pub(crate) const fn height() -> &'static str {
    HEIGHT
}
