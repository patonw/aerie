use std::{
    collections::{BTreeMap, HashMap},
    num::NonZeroUsize,
    str::FromStr as _,
    sync::{Arc, Mutex},
};

use anyhow::Context as _;
use arc_swap::{ArcSwap, ArcSwapOption};
use itertools::Itertools as _;
use lru::LruCache;
use tokio_util::sync::CancellationToken;
use typed_builder::TypedBuilder;

use crate::cli;
use aerie::{
    AgentFactory, ChatSession, Preferences, config::ModelRole, storage::CachedDirStore as _,
    toolbox::ToolStore, workflow::store::WorkflowStoreDir,
};

#[derive(Debug, Default, Clone)]
pub struct UserData {
    name: String,
    env: HashMap<String, String>,
}

impl UserData {
    #[allow(unused)]
    pub fn with_env(self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            env: env.into_iter().collect(),
            ..self
        }
    }
}

impl From<&str> for UserData {
    fn from(value: &str) -> Self {
        Self {
            name: value.to_string(),
            env: Default::default(),
        }
    }
}

#[derive(TypedBuilder)]
pub struct ScopeFactory {
    pub models: Arc<BTreeMap<ModelRole, String>>,
    pub workflow_store: Option<WorkflowStoreDir>,
    pub session: ChatSession,
    pub settings: Arc<Preferences>,
    pub rt: tokio::runtime::Handle,
    pub agent_factory: AgentFactory,
    pub shutdown: CancellationToken,

    #[builder(default=Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap())))]
    pub scopes: Mutex<LruCache<String, Arc<ExecScope>>>,
}

impl ScopeFactory {
    #[cfg(feature = "runner-http")]
    pub fn no_tools(&self) -> Arc<ExecScope> {
        self.scopes
            .lock()
            .unwrap()
            .get_or_insert("###no_tools###".into(), || {
                let agent_factory = self
                    .agent_factory
                    .clone()
                    .with_env(std::env::vars())
                    .with_tools(None);

                Arc::new(
                    ExecScope::builder()
                        .models(self.models.clone())
                        .workflow_store(self.workflow_store.clone())
                        .session(self.session.clone())
                        .settings(self.settings.clone())
                        .rt(self.rt.clone())
                        .agent_factory(agent_factory)
                        .shutdown(self.shutdown.clone())
                        .build(),
                )
            })
            .clone()
    }

    pub fn user_scope(&self, user_data: &UserData) -> Arc<ExecScope> {
        let UserData { name, env } = user_data;
        self.scopes
            .lock()
            .unwrap()
            .get_or_insert(name.clone(), || {
                let env = std::env::vars().chain(env.clone());
                let agent_factory = self.agent_factory.clone().with_env(env);

                let load_results = self.rt.block_on(async { agent_factory.load_tools().await });
                for result in load_results {
                    if let Err(err) = result {
                        tracing::error!(error = %err);
                    }
                }

                Arc::new(
                    ExecScope::builder()
                        .models(self.models.clone())
                        .workflow_store(self.workflow_store.clone())
                        .session(self.session.clone())
                        .settings(self.settings.clone())
                        .rt(self.rt.clone())
                        .agent_factory(agent_factory)
                        .shutdown(self.shutdown.clone())
                        .build(),
                )
            })
            .clone()
    }
}

/// App state common to all subcommands
#[derive(TypedBuilder, Clone)]
pub struct ExecScope {
    pub models: Arc<BTreeMap<ModelRole, String>>,
    pub workflow_store: Option<WorkflowStoreDir>,
    pub session: ChatSession,
    pub settings: Arc<Preferences>,
    pub rt: tokio::runtime::Handle,
    pub agent_factory: AgentFactory,
    pub shutdown: CancellationToken,
}

/// Instantiate and initialize common objects from global arguments
pub fn nameless_scope(
    args: &cli::Args,
    rt: &tokio::runtime::Handle,
) -> anyhow::Result<Arc<ExecScope>> {
    Ok(make_scope_factory(args, rt)?.user_scope(&UserData::default()))
}

pub fn make_scope_factory(
    args: &cli::Args,
    rt: &tokio::runtime::Handle,
) -> anyhow::Result<ScopeFactory> {
    let workflow_store = args
        .workflows
        .as_ref()
        .map(|p| WorkflowStoreDir::init(p, false))
        .transpose()?
        .context("Workflow directory required for server mode")?;
    let tool_store = args.tools.as_ref().map(|p| {
        let store = ToolStore::new(p);
        store.preload_all();
        store
    });
    let session_dir = args
        .session
        .as_ref()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    let session = ChatSession::from_dir_name(
        session_dir,
        args.session
            .as_ref()
            .and_then(|s| s.file_stem().map(|s| s.display().to_string()))
            .as_deref(),
    )
    .build()?;
    if let Some(branch) = &args.branch {
        session.transform(|history| Ok(history.switch(branch)))?;
    }
    let settings_path = if let Some(path) = &args.config {
        if !path.is_file() {
            anyhow::bail!("Configuration file does not exist: {path:?}");
        }
        path.clone()
    } else {
        Default::default()
    };
    let mut settings = if settings_path.is_file() {
        let text = std::fs::read_to_string(&settings_path)?;
        toml::from_str(&text)?
    } else {
        Preferences::default()
    };
    tracing::debug!("Loaded settings {settings:?}");
    if let Some(profile) = &args.profile {
        if !settings.has_profile(profile) {
            anyhow::bail!(
                "Profile '{profile}' not found. Valid profiles: {:?}",
                settings.models.keys().collect_vec()
            );
        }

        settings.profile = profile.to_owned();
    }
    if let Some(temperature) = &args.temperature {
        settings.temperature = *temperature;
    }
    let models: BTreeMap<ModelRole, String> = if args.models.is_empty() {
        settings.get_model_map()
    } else {
        tracing::debug!("Setting models from {:?}", &args.models);

        let mut model_map: BTreeMap<ModelRole, String> = Default::default();

        for entry in &args.models {
            tracing::debug!("Parsing model {entry}");

            if let Some((name, roles)) = entry.split_once("=") {
                for role in roles
                    .split(",")
                    .map(|r| ModelRole::from_str(r.trim()).unwrap_or_default())
                {
                    model_map.insert(role.to_owned(), name.to_string());
                }
            } else {
                model_map.insert(ModelRole::Default, entry.trim().to_string());
            }
        }

        model_map
    };
    let models = Arc::new(models);
    let next_workflow: Arc<ArcSwapOption<String>> = Default::default();
    let next_prompt: Arc<ArcSwapOption<String>> = Default::default();
    let next_images: Arc<ArcSwap<Vec<String>>> = Default::default();
    let agent_factory = AgentFactory::builder()
        .rt(rt.clone())
        .prefs(Arc::new(ArcSwap::from_pointee(settings.clone())))
        .tools(tool_store)
        .store(Some(workflow_store.clone()))
        .next_workflow(next_workflow.clone())
        .next_prompt(next_prompt.clone())
        .next_images(next_images.clone())
        .build();

    // // Now loads on new scope
    // let load_results = rt.block_on(async { agent_factory.load_tools().await });
    // for result in load_results {
    //     if let Err(err) = result {
    //         tracing::error!(error = %err);
    //     }
    // }

    Ok(ScopeFactory::builder()
        .models(models)
        .workflow_store(Some(workflow_store))
        .session(session)
        .settings(Arc::new(settings))
        .rt(rt.clone())
        .agent_factory(agent_factory)
        .shutdown(CancellationToken::new())
        .build())
}
