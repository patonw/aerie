use itertools::Itertools;
use jaq_core::{
    Ctx, Filter, Native, RcIter,
    load::{Arena, File, Loader},
};
use jaq_json::Val;
use minijinja::Environment;
use serde_json::Value;
use typed_builder::TypedBuilder;

// Pointless as a struct if we're not caching filters or templates
/// Utility for rendering templates, transforming JSON, and other data conversions
#[derive(Default, Clone, TypedBuilder)]
pub struct Transmuter {}

pub type FilterT = Filter<Native<Val>>;

impl Transmuter {
    // Using anyhow has a placeholder until we can figure out a better way to handle Jaq's errors
    pub fn init_filter(&self, filter: &str) -> anyhow::Result<FilterT> {
        let arena = Arena::default();

        let program = File {
            code: filter,
            path: (), // TODO: set this from workflow/node
        };

        let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));

        let modules = match loader.load(&arena, program) {
            Ok(value) => value,
            Err(err) => {
                anyhow::bail!("Jaq error {err:?}");
            }
        };

        let filter = match jaq_core::Compiler::default()
            .with_funs(jaq_std::funs().chain(jaq_json::funs()))
            .compile(modules)
        {
            Ok(value) => value,
            Err(err) => {
                anyhow::bail!("Jaq error {err:?}");
            }
        };

        Ok(filter)
    }

    pub fn run_filter(&self, filter: &FilterT, input: Value) -> anyhow::Result<Value> {
        let inputs = RcIter::new(core::iter::empty());

        // iterator over the output values
        let out = filter
            .run((Ctx::new([], &inputs), Val::from(input)))
            .collect_vec();

        if out.iter().any(|r| r.is_err()) {
            let errs = out
                .into_iter()
                .filter_map(|r| r.err().clone())
                .collect_vec();

            // Really starting to hate this API
            anyhow::bail!("Errors running jaq filter {:?}", errs);
        }

        let mut items = out.into_iter().filter_map(|r| r.ok()).collect_vec();
        if items.len() == 1 {
            Ok(Value::from(items.pop().unwrap()))
        } else {
            let values = items.into_iter().map(Value::from).collect_vec();
            Ok(serde_json::Value::Array(values))
        }
    }

    pub fn render_template(
        &self,
        template: &str,
        vars: &serde_json::Value,
    ) -> anyhow::Result<String> {
        // These lifetimes are tied to the template string.
        // We'd need to use a heap based collection with self_cell or yoke.
        let mut env = Environment::new();

        env.add_global(
            "CONTEXT".to_string(),
            minijinja::Value::from_serialize(vars),
        );

        let tmpl = env.template_from_str(template)?;
        // vars.as_object().unwrap()

        Ok(tmpl.render(vars)?)
    }
}
