use std::{fs::OpenOptions, path::PathBuf};

use aerie::{
    utils::message_text,
    workflow::{Value, write_value},
};

/// Worker task to dump outputs to console as a streamed JSON object
pub async fn console_output(
    out_rx: flume::Receiver<(String, Value)>,
    pretty: bool,
) -> anyhow::Result<()> {
    use aerie::workflow::Value;
    use struson::writer::*;

    let mut json_writer = JsonStreamWriter::new_custom(
        std::io::stdout(),
        WriterSettings {
            pretty_print: pretty,
            ..Default::default()
        },
    );

    json_writer.begin_object()?;
    while let Ok((label, value)) = out_rx.recv_async().await {
        json_writer.name(&label)?;

        match value {
            Value::Text(text) => {
                json_writer.string_value(&text)?;
            }
            Value::Number(value) => json_writer.fp_number_value(value.into_inner())?,
            Value::Integer(value) => json_writer.number_value(value)?,
            Value::Json(value) => json_writer.serialize_value(&value)?,
            Value::Chat(value) => json_writer.serialize_value(&value)?,
            Value::Message(message) => json_writer.string_value(&message_text(&message))?,
            Value::TextList(value) => json_writer.serialize_value(&value)?,
            Value::FloatList(value) => json_writer.serialize_value(&value)?,
            Value::IntList(value) => json_writer.serialize_value(&value)?,
            Value::MsgList(value) => json_writer.serialize_value(&value)?,
            _ => {
                json_writer.serialize_value(&value)?;
            }
        }
    }

    json_writer.end_object()?;
    json_writer.finish_document()?;
    println!();

    Ok(())
}

/// Worker task to dump workflow outputs to distinct files in a directory
pub async fn file_output(
    out_rx: flume::Receiver<(String, Value)>,
    path: PathBuf,
) -> anyhow::Result<()> {
    while let Ok((label, value)) = out_rx.recv_async().await {
        let path = path.join(label);

        let fh = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        // if out_glob.matches(&label) {
        write_value(fh, &value)?;
        // }
    }

    Ok(())
}
