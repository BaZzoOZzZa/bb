mod host_ops;
mod loader;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use deno_resolver::npm::{
    ByonmNpmResolver, ByonmNpmResolverCreateOptions, CreateInNpmPkgCheckerOptions,
    DenoInNpmPackageChecker, NpmResolver,
};
use deno_runtime::deno_core::error::JsError;
use deno_runtime::deno_core::{v8, FastString, ModuleSpecifier, PollEventLoopOptions};
use deno_runtime::deno_fs::RealFs;
use deno_runtime::deno_node::{NodeExtInitServices, NodeRequireLoader};
use deno_runtime::deno_permissions::{
    Permissions, PermissionsContainer, RuntimePermissionDescriptorParser,
};
use deno_runtime::worker::{MainWorker, WorkerOptions, WorkerServiceOptions};
use deno_runtime::BootstrapOptions;
use node_resolver::cache::NodeResolutionSys;
use node_resolver::{
    DenoIsBuiltInNodeModuleChecker, NodeResolver, NodeResolverOptions, PackageJsonResolver,
};
use sys_traits::impls::RealSys;

type Sys = RealSys;

#[cfg(feature = "snapshot")]
mod snapshot_data {
    pub static SNAPSHOT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/BB_SNAPSHOT.bin"));
    include!(concat!(env!("OUT_DIR"), "/residual_lazy.rs"));
}

struct SpikeRequireLoader;

impl NodeRequireLoader for SpikeRequireLoader {
    fn ensure_read_permission<'a>(
        &self,
        _permissions: &mut PermissionsContainer,
        path: Cow<'a, Path>,
    ) -> Result<Cow<'a, Path>, deno_error::JsErrorBox> {
        Ok(path)
    }

    fn load_text_file_lossy(
        &self,
        path: &Path,
    ) -> Result<FastString, deno_error::JsErrorBox> {
        let text = std::fs::read_to_string(path)
            .map_err(deno_error::JsErrorBox::from_err)?;
        Ok(FastString::from(text))
    }

    fn is_maybe_cjs(
        &self,
        specifier: &url::Url,
    ) -> Result<bool, node_resolver::errors::PackageJsonLoadError> {
        Ok(specifier.path().ends_with(".cjs") || specifier.path().ends_with(".js"))
    }

    fn is_maybe_cjs_from_require(
        &self,
        specifier: &url::Url,
    ) -> Result<bool, node_resolver::errors::PackageJsonLoadError> {
        Ok(!specifier.path().ends_with(".mjs"))
    }
}

fn node_services() -> NodeExtInitServices<DenoInNpmPackageChecker, NpmResolver<Sys>, Sys> {
    let sys = RealSys;
    let pkg_json_resolver = Arc::new(PackageJsonResolver::new(sys.clone(), None));
    let byonm = Arc::new(ByonmNpmResolver::new(ByonmNpmResolverCreateOptions {
        root_node_modules_dir: None,
        search_stop_dir: None,
        sys: NodeResolutionSys::new(sys.clone(), None),
        pkg_json_resolver: pkg_json_resolver.clone(),
    }));
    let node_resolver = Arc::new(NodeResolver::new(
        DenoInNpmPackageChecker::new(CreateInNpmPkgCheckerOptions::Byonm),
        DenoIsBuiltInNodeModuleChecker,
        NpmResolver::Byonm(byonm),
        pkg_json_resolver.clone(),
        NodeResolutionSys::new(sys.clone(), None),
        NodeResolverOptions::default(),
    ));
    NodeExtInitServices {
        node_require_loader: Rc::new(SpikeRequireLoader),
        node_resolver,
        pkg_json_resolver,
        sys,
    }
}

pub struct Host {
    worker: MainWorker,
}

impl Host {
    pub async fn new(data_dir: PathBuf, argv: Vec<String>) -> Result<Self> {
        let main_url = ModuleSpecifier::parse(loader::MAIN_URL)?;
        let permissions = PermissionsContainer::new(
            Arc::new(RuntimePermissionDescriptorParser::new(RealSys)),
            Permissions::allow_all(),
        );
        let services = WorkerServiceOptions::<DenoInNpmPackageChecker, NpmResolver<Sys>, Sys> {
            blob_store: Arc::new(deno_runtime::deno_web::BlobStore::default()),
            broadcast_channel: Default::default(),
            deno_rt_native_addon_loader: None,
            feature_checker: Default::default(),
            fs: Arc::new(RealFs),
            module_loader: Rc::new(loader::BbModuleLoader),
            node_services: Some(node_services()),
            npm_process_state_provider: None,
            permissions,
            root_cert_store_provider: None,
            fetch_dns_resolver: Default::default(),
            shared_array_buffer_store: None,
            compiled_wasm_module_store: None,
            v8_code_cache: None,
            bundle_provider: None,
        };
        let options = WorkerOptions {
            bootstrap: BootstrapOptions {
                args: argv,
                ..Default::default()
            },
            extensions: vec![host_ops::bb_host::init(host_ops::HostState::open(data_dir)?)],
            #[cfg(feature = "snapshot")]
            startup_snapshot: Some(snapshot_data::SNAPSHOT),
            #[cfg(feature = "snapshot")]
            residual_lazy_js_sources: snapshot_data::RESIDUAL_LAZY_JS,
            #[cfg(feature = "snapshot")]
            residual_lazy_esm_sources: snapshot_data::RESIDUAL_LAZY_ESM,
            ..Default::default()
        };
        let mut worker = MainWorker::bootstrap_from_options(&main_url, services, options);
        worker
            .execute_main_module(&main_url)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        Ok(Self { worker })
    }

    pub async fn eval_json(&mut self, expression: &str) -> Result<serde_json::Value> {
        let source = format!(
            "(async () => {{ const value = await ({expression}); return JSON.stringify(value ?? null); }})()"
        );
        let runtime = &mut self.worker.js_runtime;
        let promise = runtime
            .execute_script("<bb-host-call>", source)
            .map_err(|error| anyhow!("{error}"))?;
        let resolved = runtime.resolve(promise);
        let global = runtime
            .with_event_loop_promise(resolved, PollEventLoopOptions::default())
            .await
            .map_err(|error| anyhow!("{}", describe_js_error(&error)))?;
        deno_runtime::deno_core::scope!(scope, runtime);
        let local = v8::Local::new(scope, global);
        let text = local.to_rust_string_lossy(scope);
        serde_json::from_str(&text).context("host call returned invalid json")
    }

    pub async fn run_event_loop_for(&mut self, duration: std::time::Duration) -> Result<()> {
        let fut = self.worker.run_event_loop(false);
        match tokio::time::timeout(duration, fut).await {
            Ok(result) => result.map_err(|error| anyhow!("{error}")),
            Err(_) => Ok(()),
        }
    }

    pub async fn run_event_loop(&mut self) -> Result<()> {
        self.worker
            .run_event_loop(false)
            .await
            .map_err(|error| anyhow!("{error}"))
    }
}

fn describe_js_error(error: &deno_runtime::deno_core::error::CoreError) -> String {
    match error.as_kind() {
        deno_runtime::deno_core::error::CoreErrorKind::Js(js) => format_js(js),
        _ => error.to_string(),
    }
}

fn format_js(js: &JsError) -> String {
    js.to_string()
}

fn rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
}

fn js_str(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

fn plugin_entry(assets: &Path, plugin: &str) -> Result<String> {
    let path = assets.join("plugins").join(plugin).join("dist").join("server.js");
    let path = path
        .canonicalize()
        .with_context(|| format!("missing {}", path.display()))?;
    Ok(url::Url::from_file_path(path).unwrap().to_string())
}

async fn step(host: &mut Host, label: &str, expression: &str) -> serde_json::Value {
    let started = Instant::now();
    let result = host.eval_json(expression).await;
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    match result {
        Ok(value) => {
            println!(
                "[{label}] ok in {elapsed:.1}ms: {}",
                truncate(&value.to_string(), 1200)
            );
            value
        }
        Err(error) => {
            println!("[{label}] FAILED in {elapsed:.1}ms: {error:#}");
            serde_json::Value::Null
        }
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}... ({} bytes)", &text[..max], text.len())
    }
}

async fn run_suite(assets: PathBuf, data_dir: PathBuf) -> Result<()> {
    let boot = Instant::now();
    let mut host = Host::new(data_dir, vec![]).await?;
    println!(
        "[boot] runtime ready in {:.1}ms, rss={:?}kB",
        boot.elapsed().as_secs_f64() * 1000.0,
        rss_kb()
    );

    step(&mut host, "node-probe", "globalThis.__bb.probeNode()").await;
    println!("[rss] after node probe: {:?}kB", rss_kb());

    let thread_list = plugin_entry(&assets, "thread-list")?;
    step(
        &mut host,
        "thread-list.load",
        &format!("globalThis.__bb.loadPlugin('thread-list', {})", js_str(&thread_list)),
    )
    .await;
    step(&mut host, "thread-list.rpc.listPreferences", "globalThis.__bb.invokeRpc('thread-list', 'listPreferences', null)").await;
    step(&mut host, "thread-list.rpc.setPreference", "globalThis.__bb.invokeRpc('thread-list', 'setPreference', { key: 'chronologicalSort', value: 'created' })").await;
    step(&mut host, "thread-list.rpc.setPreference(invalid)", "globalThis.__bb.invokeRpc('thread-list', 'setPreference', { key: 'chronologicalSort', value: 42 })").await;
    step(&mut host, "thread-list.cli.prefs-get", "globalThis.__bb.invokeCli('thread-list', ['prefs', 'get', 'chronologicalSort', '--json'], {})").await;
    step(&mut host, "thread-list.cli.prefs-list", "globalThis.__bb.invokeCli('thread-list', ['prefs', 'list'], {})").await;

    let automations = plugin_entry(&assets, "automations")?;
    step(
        &mut host,
        "automations.load",
        &format!("globalThis.__bb.loadPlugin('automations', {})", js_str(&automations)),
    )
    .await;
    step(&mut host, "automations.rpc.overview", "globalThis.__bb.invokeRpc('automations', 'automations_overview', null)").await;
    step(&mut host, "automations.rpc.list", "globalThis.__bb.invokeRpc('automations', 'automations_list', { projectId: 'proj_spike' })").await;
    step(&mut host, "automations.event.thread.deleted", "globalThis.__bb.emitEvent('automations', 'thread.deleted', { thread: { id: 'thr_spike' } })").await;
    step(&mut host, "automations.services.start", "globalThis.__bb.startServices('automations')").await;
    let loop_started = Instant::now();
    host.run_event_loop_for(std::time::Duration::from_millis(1500)).await?;
    println!("[automations.services] event loop pumped for {:.0}ms", loop_started.elapsed().as_secs_f64() * 1000.0);
    step(&mut host, "automations.services.stop", "globalThis.__bb.stopServices('automations')").await;
    step(&mut host, "automations.cli.help", "globalThis.__bb.invokeCli('automations', ['list', '--project', 'proj_spike', '--json'], {})").await;

    let secrets = plugin_entry(&assets, "secrets")?;
    step(
        &mut host,
        "secrets.load",
        &format!("globalThis.__bb.loadPlugin('secrets', {})", js_str(&secrets)),
    )
    .await;
    step(&mut host, "secrets.cli.request", "globalThis.__bb.invokeCli('secrets', ['request', 'API_KEY', '--write-env', '.env'], { cwd: '/tmp', threadId: 'thr_spike' })").await;

    let claude = plugin_entry(&assets, "provider-claude-code")?;
    step(
        &mut host,
        "provider-claude-code.load",
        &format!("globalThis.__bb.loadPlugin('provider-claude-code', {})", js_str(&claude)),
    )
    .await;
    let host_artifact = assets
        .join("plugins/provider-claude-code/dist/host.js")
        .canonicalize()?;
    let host_url = url::Url::from_file_path(host_artifact).unwrap().to_string();
    step(
        &mut host,
        "provider-claude-code.host-artifact.import",
        &format!(
            "import({}).then((m) => ({{ exports: Object.keys(m), bridgeApiVersion: m.experimental_providerBridge?.experimental_apiVersion, bridgeHandlers: Object.keys(m.experimental_providerBridge ?? {{}}), defaultKeys: Object.keys(m.default ?? {{}}) }}))",
            js_str(&host_url)
        ),
    )
    .await;
    step(
        &mut host,
        "provider-claude-code.host-artifact.resolveNativeRoots",
        &format!(
            "import({}).then((m) => m.default.handlers.resolveNativeRoots({{ cwd: '/tmp' }}))",
            js_str(&host_url)
        ),
    )
    .await;

    println!("[records]");
    for record in host_ops::take_records(&mut host.worker.js_runtime) {
        println!("  {}", truncate(&record, 400));
    }
    println!(
        "[done] total {:.1}ms, rss={:?}kB",
        boot.elapsed().as_secs_f64() * 1000.0,
        rss_kb()
    );
    Ok(())
}

async fn run_bridge(assets: PathBuf, data_dir: PathBuf, plugin: String) -> Result<()> {
    let host_js = assets
        .join("plugins")
        .join(&plugin)
        .join("dist/host.js")
        .canonicalize()?;
    std::fs::create_dir_all(&data_dir)?;
    let argv = vec![
        host_js.display().to_string(),
        plugin.clone(),
        data_dir.canonicalize()?.display().to_string(),
    ];
    let mut host = Host::new(data_dir, argv).await?;
    host.eval_json("import('file:///__bb_internal__/provider-bridge-worker-entry.mjs').then(() => true)")
        .await?;
    host.run_event_loop().await
}

async fn run_sweep(assets: PathBuf, data_dir: PathBuf) -> Result<()> {
    let mut host = Host::new(data_dir, vec![]).await?;
    let mut names: Vec<String> = std::fs::read_dir(assets.join("plugins"))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let (mut ok, mut failed) = (0, 0);
    for name in names {
        let dist = assets.join("plugins").join(&name).join("dist");
        let to_url = |file: &str| -> Option<String> {
            let path = dist.join(file);
            path.canonicalize()
                .ok()
                .map(|path| url::Url::from_file_path(path).unwrap().to_string())
        };
        let server = to_url("server.js");
        let host_artifact = to_url("host.js");
        let expression = format!(
            "globalThis.__bb.sweepPlugin({}, {}, {})",
            js_str(&name),
            server.as_deref().map(js_str).unwrap_or_else(|| "null".into()),
            host_artifact.as_deref().map(js_str).unwrap_or_else(|| "null".into()),
        );
        let report = host.eval_json(&expression).await?;
        let has_error = report.get("serverError").is_some() || report.get("hostError").is_some();
        if has_error { failed += 1 } else { ok += 1 }
        println!("{} {}", if has_error { "FAIL" } else { "ok  " }, truncate(&report.to_string(), 700));
    }
    println!("[sweep] ok={ok} failed={failed} rss={:?}kB", rss_kb());
    Ok(())
}

async fn run_services(assets: PathBuf, data_dir: PathBuf, plugins: Vec<String>) -> Result<()> {
    let mut host = Host::new(data_dir, vec![]).await?;
    for plugin in &plugins {
        let entry = plugin_entry(&assets, plugin)?;
        step(&mut host, &format!("{plugin}.load"), &format!("globalThis.__bb.loadPlugin({}, {})", js_str(plugin), js_str(&entry))).await;
        step(&mut host, &format!("{plugin}.services.start"), &format!("globalThis.__bb.startServices({})", js_str(plugin))).await;
    }
    host.run_event_loop_for(std::time::Duration::from_secs(4)).await?;
    for plugin in &plugins {
        step(&mut host, &format!("{plugin}.services.stop"), &format!("globalThis.__bb.stopServices({})", js_str(plugin))).await;
    }
    host.run_event_loop_for(std::time::Duration::from_millis(500)).await?;
    println!("[services] rss={:?}kB", rss_kb());
    Ok(())
}

async fn run_boot_only(data_dir: PathBuf) -> Result<()> {
    let boot = Instant::now();
    let mut host = Host::new(data_dir, vec![]).await?;
    let value = host.eval_json("1 + 1").await?;
    println!(
        "boot+eval {:.1}ms value={value} rss={:?}kB",
        boot.elapsed().as_secs_f64() * 1000.0,
        rss_kb()
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).cloned().unwrap_or_else(|| "suite".to_string());
    let assets = PathBuf::from(
        std::env::var("BB_SPIKE_ASSETS")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/assets").to_string()),
    );
    let data_dir = PathBuf::from(
        std::env::var("BB_SPIKE_DATA").unwrap_or_else(|_| {
            std::env::temp_dir()
                .join(format!("bb-plugin-host-spike-{}", std::process::id()))
                .display()
                .to_string()
        }),
    );
    std::fs::create_dir_all(&data_dir)?;
    deno_runtime::tokio_util::create_and_run_current_thread(async move {
        match mode.as_str() {
            "suite" => run_suite(assets, data_dir).await,
            "bridge" => {
                let plugin = args.get(2).cloned().unwrap_or_else(|| "provider-claude-code".into());
                run_bridge(assets, data_dir, plugin).await
            }
            "boot" => run_boot_only(data_dir).await,
            "sweep" => run_sweep(assets, data_dir).await,
            "services" => run_services(assets, data_dir, args[2..].to_vec()).await,
            other => Err(anyhow!("unknown mode {other}")),
        }
    })
}
