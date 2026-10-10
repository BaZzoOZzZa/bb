use deno_error::JsErrorBox;
use deno_runtime::deno_core::error::ModuleLoaderError;
use deno_runtime::deno_core::{
    resolve_import, ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader,
    ModuleResolveResponse, ModuleSource, ModuleSourceCode, ModuleSpecifier, ModuleType,
    ResolutionKind,
};
use node_resolver::DENO_SUPPORTED_BUILTIN_NODE_MODULES;

pub const INTERNAL_PREFIX: &str = "file:///__bb_internal__/";
pub const MAIN_URL: &str = "file:///__bb_internal__/main.js";

const MAIN_JS: &str = "import * as host from \"file:///__bb_internal__/host.js\";\nglobalThis.__bb = host;\n";
const HOST_JS: &str = include_str!("../js/host.js");
const PLUGIN_SDK_JS: &str = include_str!("../assets/plugin-sdk.mjs");
const ZOD_JS: &str = include_str!("../assets/zod.mjs");
const BRIDGE_ENTRY_JS: &str = include_str!("../assets/provider-bridge-worker-entry.mjs");

fn internal_source(name: &str) -> Option<&'static str> {
    match name {
        "main.js" => Some(MAIN_JS),
        "host.js" => Some(HOST_JS),
        "plugin-sdk.mjs" => Some(PLUGIN_SDK_JS),
        "zod.mjs" => Some(ZOD_JS),
        "provider-bridge-worker-entry.mjs" => Some(BRIDGE_ENTRY_JS),
        _ => None,
    }
}

pub struct BbModuleLoader;

fn internal(name: &str) -> ModuleResolveResponse {
    ModuleSpecifier::parse(&format!("{INTERNAL_PREFIX}{name}")).map_err(JsErrorBox::from_err)
}

impl ModuleLoader for BbModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> ModuleResolveResponse {
        match specifier {
            "@get-bb/plugin-sdk" | "@bb/plugin-sdk" => return internal("plugin-sdk.mjs"),
            "zod" => return internal("zod.mjs"),
            "bb:host" => return internal("host.js"),
            _ => {}
        }
        if let Some(name) = specifier.strip_prefix("node:") {
            return ModuleSpecifier::parse(&format!("node:{name}")).map_err(JsErrorBox::from_err);
        }
        let base = specifier.split('/').next().unwrap_or(specifier);
        if DENO_SUPPORTED_BUILTIN_NODE_MODULES.contains(&specifier)
            || (DENO_SUPPORTED_BUILTIN_NODE_MODULES.contains(&base) && !specifier.starts_with('.'))
        {
            return ModuleSpecifier::parse(&format!("node:{specifier}"))
                .map_err(JsErrorBox::from_err);
        }
        let is_relative = specifier.starts_with("./")
            || specifier.starts_with("../")
            || specifier.starts_with('/')
            || specifier.contains("://")
            || specifier.starts_with("file:");
        if !is_relative {
            return Err(JsErrorBox::generic(format!(
                "bare specifier \"{specifier}\" is not provided by the bb plugin host (from {referrer})"
            )));
        }
        resolve_import(specifier, referrer).map_err(JsErrorBox::from_err)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleLoadReferrer>,
        _options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let specifier = module_specifier.clone();
        let result = (|| -> Result<ModuleSource, ModuleLoaderError> {
            let text = specifier.as_str();
            if let Some(name) = text.strip_prefix(INTERNAL_PREFIX) {
                let source = internal_source(name).ok_or_else(|| {
                    JsErrorBox::generic(format!("unknown internal module {name}"))
                })?;
                return Ok(ModuleSource::new(
                    ModuleType::JavaScript,
                    ModuleSourceCode::String(source.to_string().into()),
                    &specifier,
                    None,
                ));
            }
            if specifier.scheme() != "file" {
                return Err(JsErrorBox::generic(format!(
                    "unsupported module scheme: {specifier}"
                )));
            }
            let path = specifier
                .to_file_path()
                .map_err(|_| JsErrorBox::generic(format!("invalid file url {specifier}")))?;
            let module_type = match path.extension().and_then(|ext| ext.to_str()) {
                Some("json") => ModuleType::Json,
                _ => ModuleType::JavaScript,
            };
            let code = std::fs::read(&path).map_err(|error| {
                JsErrorBox::generic(format!("failed to read {}: {error}", path.display()))
            })?;
            Ok(ModuleSource::new(
                module_type,
                ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
                &specifier,
                None,
            ))
        })();
        ModuleLoadResponse::Sync(result)
    }
}
