//! Define `UniversalArtifact`, based on `UniversalArtifactBuild`
//! to allow compiling and instantiating to be done as separate steps.

use crate::engine::{UniversalEngine, UniversalEngineInner};
use crate::link::link_module;
use enumset::EnumSet;
use std::convert::TryFrom;
use std::sync::{Arc, Mutex};
#[cfg(feature = "compiler")]
use wasmer_compiler::ModuleEnvironment;
use wasmer_compiler::{CpuFeature, Features, Triple};
#[cfg(feature = "compiler")]
use wasmer_engine::Tunables;
use wasmer_engine::{register_frame_info, Artifact, FunctionExtent, GlobalFrameInfoRegistration};
use wasmer_engine_universal_artifact::ArtifactCreate;
use wasmer_engine_universal_artifact::{UniversalArtifactBuild, UniversalArtifactBuildRef};
use wasmer_types::entity::{BoxedSlice, PrimaryMap, PrimaryMapRef};
use wasmer_types::{
    CompileError, DataInitializer, DeserializeError, FunctionIndex, LocalFunctionIndex,
    MemoryIndex, ModuleInfo, SerializeError, SignatureIndex, TableIndex,
};
use wasmer_vm::{
    FuncDataRegistry, FunctionBodyPtr, MemoryStyle, TableStyle, VMSharedSignatureIndex,
    VMTrampoline,
};

/// A compiled wasm module, ready to be instantiated.
pub struct UniversalArtifact {
    artifact: UniversalArtifactBuild,
    finished_functions: BoxedSlice<LocalFunctionIndex, FunctionBodyPtr>,
    finished_function_call_trampolines: BoxedSlice<SignatureIndex, VMTrampoline>,
    finished_dynamic_function_trampolines: BoxedSlice<FunctionIndex, FunctionBodyPtr>,
    signatures: BoxedSlice<SignatureIndex, VMSharedSignatureIndex>,
    func_data_registry: Arc<FuncDataRegistry>,
    frame_info_registration: Mutex<Option<GlobalFrameInfoRegistration>>,
    finished_function_lengths: BoxedSlice<LocalFunctionIndex, usize>,
}

impl UniversalArtifact {
    /// Compile a data buffer into a `UniversalArtifactBuild`, which may then be instantiated.
    #[cfg(feature = "compiler")]
    pub fn new_build(
        engine: &UniversalEngine,
        data: &[u8],
        tunables: &dyn Tunables,
    ) -> Result<UniversalArtifactBuild, CompileError> {
        let environ = ModuleEnvironment::new();
        let mut inner_engine = engine.inner_mut();
        let translation = environ.translate(data).map_err(CompileError::Wasm)?;
        let module = translation.module;
        let memory_styles: PrimaryMap<MemoryIndex, MemoryStyle> = module
            .memories
            .values()
            .map(|memory_type| tunables.memory_style(memory_type))
            .collect();
        let table_styles: PrimaryMap<TableIndex, TableStyle> = module
            .tables
            .values()
            .map(|table_type| tunables.table_style(table_type))
            .collect();

        Ok(UniversalArtifactBuild::new(
            inner_engine.builder_mut(),
            data,
            engine.target(),
            memory_styles,
            table_styles,
        )?)
    }

    /// Compile a data buffer into a `UniversalArtifactBuild`, which may then be instantiated.
    #[cfg(not(feature = "compiler"))]
    pub fn new_build(_engine: &UniversalEngine, _data: &[u8]) -> Result<Self, CompileError> {
        Err(CompileError::Codegen(
            "Compilation is not enabled in the engine".to_string(),
        ))
    }

    /// Deserialize a UniversalArtifactBuild
    ///
    /// # Safety
    /// This function is unsafe because rkyv reads directly without validating
    /// the data.
    pub unsafe fn deserialize(
        engine: &UniversalEngine,
        bytes: &[u8],
    ) -> Result<Self, DeserializeError> {
        /*
        let bytes = &bytes[UniversalArtifactBuild::MAGIC_HEADER.len()..];
        let metadata_len = MetadataHeader::parse(bytes)?;
        let metadata_slice: &[u8] = &bytes[MetadataHeader::LEN..][..metadata_len];
        let serializable = SerializableModule::deserialize(metadata_slice)?;
        let artifact = UniversalArtifactBuild::from_serializable(serializable);
        */
        let mut inner_engine = engine.inner_mut();
        let build_ref = UniversalArtifactBuildRef::try_from(bytes)?;
        Self::from_build(&mut inner_engine, build_ref).map_err(DeserializeError::Compiler)
    }

    /// Construct a `UniversalArtifactBuild` from component parts.
    pub fn from_build<'a>(
        engine_inner: &mut UniversalEngineInner,
        artifact: UniversalArtifactBuildRef<'a>,
    ) -> Result<Self, CompileError> {
        let module_info = artifact.create_module_info();
        let custom_sections_artifact = artifact.get_custom_sections();
        let (
            finished_functions,
            finished_function_call_trampolines,
            finished_dynamic_function_trampolines,
            custom_sections,
        ) = engine_inner.allocate(
            &module_info,
            artifact.get_function_bodies_ref().into(),
            artifact.get_function_call_trampolines_ref().into(),
            artifact.get_dynamic_function_trampolines_ref().into(),
            &custom_sections_artifact,
        )?;

        link_module(
            &module_info,
            &finished_functions,
            artifact.get_function_relocations_ref().into(),
            &custom_sections,
            artifact.get_custom_section_relocations_ref().into(),
            artifact.get_libcall_trampolines(),
            artifact.get_libcall_trampoline_len(),
        );

        // Compute indices into the shared signature table.
        let signatures = {
            let signature_registry = engine_inner.signatures();
            module_info
                .signatures
                .values()
                .map(|sig| signature_registry.register(sig))
                .collect::<PrimaryMap<_, _>>()
        };

        let eh_frame = match artifact.get_debug_ref() {
            Some(debug) => {
                let eh_frame_section_size = custom_sections_artifact[debug.eh_frame].bytes.len();
                let eh_frame_section_pointer = custom_sections[debug.eh_frame];
                Some(unsafe {
                    std::slice::from_raw_parts(*eh_frame_section_pointer, eh_frame_section_size)
                })
            }
            None => None,
        };

        // Make all code compiled thus far executable.
        engine_inner.publish_compiled_code();

        engine_inner.publish_eh_frame(eh_frame)?;

        let finished_function_lengths = finished_functions
            .values()
            .map(|extent| extent.length)
            .collect::<PrimaryMap<LocalFunctionIndex, usize>>()
            .into_boxed_slice();
        let finished_functions = finished_functions
            .values()
            .map(|extent| extent.ptr)
            .collect::<PrimaryMap<LocalFunctionIndex, FunctionBodyPtr>>()
            .into_boxed_slice();
        let finished_function_call_trampolines =
            finished_function_call_trampolines.into_boxed_slice();
        let finished_dynamic_function_trampolines =
            finished_dynamic_function_trampolines.into_boxed_slice();
        let signatures = signatures.into_boxed_slice();
        let func_data_registry = engine_inner.func_data().clone();

        Ok(Self {
            artifact: UniversalArtifactBuild::try_from(artifact)
                .map_err(|err| CompileError::Validate(err.to_string()))?,
            finished_functions,
            finished_function_call_trampolines,
            finished_dynamic_function_trampolines,
            signatures,
            frame_info_registration: Mutex::new(None),
            finished_function_lengths,
            func_data_registry,
        })
    }

    /// Get the default extension when serializing this artifact
    pub fn get_default_extension(triple: &Triple) -> &'static str {
        UniversalArtifactBuild::get_default_extension(triple)
    }

    /// Check if the provided bytes look like a serialized `UniversalArtifactBuild`.
    pub fn is_deserializable(bytes: &[u8]) -> bool {
        UniversalArtifactBuild::is_deserializable(bytes)
    }
}

impl ArtifactCreate for UniversalArtifact {
    fn create_module_info(&self) -> ModuleInfo {
        self.artifact.create_module_info()
    }

    fn features(&self) -> &Features {
        self.artifact.features()
    }

    fn cpu_features(&self) -> EnumSet<CpuFeature> {
        self.artifact.cpu_features()
    }

    fn data_initializers<'data>(&'data self) -> Vec<DataInitializer<'data>> {
        self.artifact.data_initializers()
    }

    fn memory_styles(&self) -> PrimaryMapRef<MemoryIndex, MemoryStyle> {
        self.artifact.memory_styles()
    }

    fn table_styles(&self) -> PrimaryMapRef<TableIndex, TableStyle> {
        self.artifact.table_styles()
    }

    fn serialize(&self) -> Result<Vec<u8>, SerializeError> {
        self.artifact.serialize()
    }
}

impl Artifact for UniversalArtifact {
    fn register_frame_info(&self) {
        let mut info = self.frame_info_registration.lock().unwrap();

        if info.is_some() {
            return;
        }

        let finished_function_extents = self
            .finished_functions
            .values()
            .copied()
            .zip(self.finished_function_lengths.values().copied())
            .map(|(ptr, length)| FunctionExtent { ptr, length })
            .collect::<PrimaryMap<LocalFunctionIndex, _>>()
            .into_boxed_slice();

        let frame_infos = self.artifact.get_frame_info_ref();
        *info = register_frame_info(
            self.artifact.create_module_info(),
            &finished_function_extents,
            frame_infos.into(),
        );
    }

    fn finished_functions(&self) -> &BoxedSlice<LocalFunctionIndex, FunctionBodyPtr> {
        &self.finished_functions
    }

    fn finished_function_call_trampolines(&self) -> &BoxedSlice<SignatureIndex, VMTrampoline> {
        &self.finished_function_call_trampolines
    }

    fn finished_dynamic_function_trampolines(&self) -> &BoxedSlice<FunctionIndex, FunctionBodyPtr> {
        &self.finished_dynamic_function_trampolines
    }

    fn signatures(&self) -> &BoxedSlice<SignatureIndex, VMSharedSignatureIndex> {
        &self.signatures
    }

    fn func_data_registry(&self) -> &FuncDataRegistry {
        &self.func_data_registry
    }
}
