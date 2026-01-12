#![cfg(feature = "migration")]
use std::{borrow::Cow, fs::File, io::Write as _, path::PathBuf};

use clap::Parser;
use im::ordmap;
use saphyr::{LoadableYamlNode, Scalar, Yaml, YamlEmitter};
use std::cell::LazyCell;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// The workflow file to migrate
    workflow: Vec<PathBuf>,
}

#[allow(clippy::declare_interior_mutable_const)]
const TAG_MAP: LazyCell<im::OrdMap<&str, &str>> = LazyCell::new(|| {
    ordmap! {
        "Agent" => "AgentNode",
        "Comment" => "CommentNode",
        "Output" => "OutputNode",
        "GraftChat" => "GraftHistory",
        "MaskChat" => "MaskHistory",
        "Context" => "ChatContext",
        "Chat" => "ChatNode",
        "Structured" => "StructuredChat"
    }
});

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    for path in &args.workflow {
        let input = std::fs::read_to_string(path)?;
        let docs = Yaml::load_from_str(&input).unwrap();
        let doc = &docs[0]; // select the first YAML document
        // dbg!(&doc);

        let out_doc = tag_to_field(doc);

        let mut out_str = String::new();
        let mut emitter = YamlEmitter::new(&mut out_str);
        emitter.multiline_strings(true);
        emitter.dump(&out_doc).unwrap(); // dump the YAML object to a String

        let mut file = File::create(path)?;
        write!(file, "{out_str}")?;
    }

    Ok(())
}

fn tag_to_field<'a>(yaml: &saphyr::Yaml<'a>) -> saphyr::Yaml<'a> {
    match yaml {
        Yaml::Sequence(yamls) => {
            let items = yamls.iter().map(tag_to_field).collect();
            Yaml::Sequence(items)
        }
        Yaml::Mapping(data) => {
            let entries = data.iter().map(|(k, v)| (k.clone(), tag_to_field(v)));
            Yaml::Mapping(entries.collect())
        }
        Yaml::Tagged(tag, yaml) => {
            #[allow(clippy::borrow_interior_mutable_const)]
            let name = TAG_MAP
                .get(tag.suffix.as_str())
                .cloned()
                .unwrap_or(tag.suffix.as_str());
            let name = Yaml::Value(Scalar::String(Cow::Owned(name.to_string())));
            let entries = std::iter::once((name, tag_to_field(yaml)));
            Yaml::Mapping(entries.collect())
        }
        other => other.clone(),
    }
}
