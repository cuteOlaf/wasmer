//! Common module with common used structures across different
//! commands.

use crate::common::WasmFeatures;
use anyhow::{Error, Result};
use std::str::FromStr;
use std::string::ToString;
use std::sync::Arc;
use structopt::StructOpt;
use wasmer::*;
use wasmer_compiler::CompilerConfig;

#[derive(Debug, Clone, StructOpt)]
/// The compiler options
pub struct StoreOptions {
    /// Use Singlepass compiler
    #[structopt(long, conflicts_with_all = &["cranelift", "llvm", "backend"])]
    singlepass: bool,

    /// Use Cranelift compiler
    #[structopt(long, conflicts_with_all = &["singlepass", "llvm", "backend"])]
    cranelift: bool,

    /// Use LLVM compiler
    #[structopt(long, conflicts_with_all = &["singlepass", "cranelift", "backend"])]
    llvm: bool,

    /// The deprecated backend flag - Please not use
    #[structopt(long = "backend", hidden = true, conflicts_with_all = &["singlepass", "cranelift", "llvm"])]
    backend: Option<String>,

    #[structopt(flatten)]
    features: WasmFeatures,
    // #[structopt(flatten)]
    // llvm_options: LLVMCLIOptions,
}

#[derive(Debug)]
enum Compiler {
    Singlepass,
    Cranelift,
    LLVM,
}

impl ToString for Compiler {
    fn to_string(&self) -> String {
        match self {
            Self::Singlepass => "singlepass".to_string(),
            Self::Cranelift => "cranelift".to_string(),
            Self::LLVM => "llvm".to_string(),
        }
    }
}

impl FromStr for Compiler {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "singlepass" => Ok(Self::Singlepass),
            "cranelift" => Ok(Self::Cranelift),
            "llvm" => Ok(Self::LLVM),
            backend => bail!("The `{}` compiler does not exist.", backend),
        }
    }
}

#[cfg(all(feature = "compiler", feature = "engine"))]
impl StoreOptions {
    fn get_compiler(&self) -> Result<Compiler> {
        if self.cranelift {
            Ok(Compiler::Cranelift)
        } else if self.llvm {
            Ok(Compiler::LLVM)
        } else if self.singlepass {
            Ok(Compiler::Singlepass)
        } else if let Some(backend) = self.backend.clone() {
            warning!(
                "the `--backend={0}` flag is deprecated, please use `--{0}` instead",
                backend
            );
            Compiler::from_str(&backend)
        } else {
            // Auto mode, we choose the best compiler for that platform
            cfg_if::cfg_if! {
                if #[cfg(all(feature = "cranelift", target_arch = "x86_64"))] {
                    return Ok(Compiler::Cranelift);
                }
                else if #[cfg(all(feature = "singlepass", target_arch = "x86_64"))] {
                    return Ok(Compiler::Singlepass);
                }
                else if #[cfg(feature = "llvm")] {
                    return Ok(Compiler::LLVM);
                } else {
                    bail!("There are no available compilers for your architecture");
                }
            }
        }
    }

    /// Get the Compiler Config for the current options
    #[allow(unused_variables)]
    fn get_config(&self, compiler: Compiler) -> Result<Box<dyn CompilerConfig>> {
        let config: Box<dyn CompilerConfig> = match compiler {
            #[cfg(feature = "singlepass")]
            Compiler::Singlepass => {
                let config = wasmer_compiler_singlepass::SinglepassConfig::default();
                Box::new(config)
            }
            #[cfg(feature = "cranelift")]
            Compiler::Cranelift => {
                let config = wasmer_compiler_cranelift::CraneliftConfig::default();
                Box::new(config)
            }
            #[cfg(feature = "llvm")]
            Compiler::LLVM => {
                let config = wasmer_compiler_llvm::LLVMConfig::default();
                Box::new(config)
            }
            #[cfg(not(all(feature = "singlepass", feature = "cranelift", feature = "llvm",)))]
            compiler => bail!(
                "The `{}` compiler is not included in this binary.",
                compiler.to_string()
            ),
        };
        Ok(config)
    }

    /// Gets the compiler config
    fn get_compiler_config(&self) -> Result<(Box<dyn CompilerConfig>, String)> {
        let compiler = self.get_compiler()?;
        let compiler_name = compiler.to_string();
        let compiler_config = self.get_config(compiler)?;
        Ok((compiler_config, compiler_name))
    }

    /// Gets the tunables for the compiler target
    pub fn get_tunables(&self, compiler_config: &dyn CompilerConfig) -> Tunables {
        Tunables::for_target(compiler_config.target().triple())
    }

    /// Gets the store
    pub fn get_store(&self) -> Result<(Store, String)> {
        let (compiler_config, compiler_name) = self.get_compiler_config()?;
        let tunables = self.get_tunables(&*compiler_config);
        #[cfg(feature = "jit")]
        let engine = wasmer_engine_jit::JITEngine::new(&*compiler_config, tunables);
        let store = Store::new(Arc::new(engine));
        Ok((store, compiler_name))
    }
}

// If we don't have a compiler, but we have an engine
#[cfg(all(not(feature = "compiler"), feature = "engine"))]
impl StoreOptions {
    /// Get the store (headless engine)
    pub fn get_store(&self) -> Result<(Store, String)> {
        // Get the tunables for the current host
        let tunables = Tunables::default();
        let engine = Engine::headless(tunables);
        let store = Store::new(&engine);
        Ok((store, "headless".to_string()))
    }
}

// If we don't have any engine enabled
#[cfg(not(feature = "engine"))]
impl StoreOptions {
    /// Get the store (headless engine)
    pub fn get_store(&self) -> Result<(Store, String)> {
        bail!("No engines are enabled");
    }
}
