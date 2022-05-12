use crate::entity::PrimaryMap;
use crate::{
    CompileModuleInfo, CompiledFunctionFrameInfo, CustomSection, DeserializeError, Dwarf,
    FunctionBody, FunctionIndex, LocalFunctionIndex, OwnedDataInitializer, Relocation,
    SectionIndex, SerializeError, SignatureIndex,
};
use rkyv::{
    de::deserializers::SharedDeserializeMap, ser::serializers::AllocSerializer,
    ser::Serializer as RkyvSerializer, Archive, Deserialize as RkyvDeserialize,
    Serialize as RkyvSerialize,
};

/// The compilation related data for a serialized modules
#[derive(Archive, RkyvDeserialize, RkyvSerialize)]
#[allow(missing_docs)]
pub struct SerializableCompilation {
    pub function_bodies: PrimaryMap<LocalFunctionIndex, FunctionBody>,
    pub function_relocations: PrimaryMap<LocalFunctionIndex, Vec<Relocation>>,
    pub function_frame_info: PrimaryMap<LocalFunctionIndex, CompiledFunctionFrameInfo>,
    pub function_call_trampolines: PrimaryMap<SignatureIndex, FunctionBody>,
    pub dynamic_function_trampolines: PrimaryMap<FunctionIndex, FunctionBody>,
    pub custom_sections: PrimaryMap<SectionIndex, CustomSection>,
    pub custom_section_relocations: PrimaryMap<SectionIndex, Vec<Relocation>>,
    // The section indices corresponding to the Dwarf debug info
    pub debug: Option<Dwarf>,
    // Custom section containing libcall trampolines.
    pub libcall_trampolines: SectionIndex,
    // Length of each libcall trampoline.
    pub libcall_trampoline_len: u32,
}

/// Serializable struct that is able to serialize from and to
/// a `UniversalArtifactInfo`.
#[derive(Archive, RkyvDeserialize, RkyvSerialize)]
#[allow(missing_docs)]
pub struct SerializableModule {
    /// The main serializable compilation object
    pub compilation: SerializableCompilation,
    /// Compilation informations
    pub compile_info: CompileModuleInfo,
    /// Datas initializers
    pub data_initializers: Box<[OwnedDataInitializer]>,
    /// CPU Feature flags for this compilation
    pub cpu_features: u64,
}

fn to_serialize_error(err: impl std::error::Error) -> SerializeError {
    SerializeError::Generic(format!("{}", err))
}

impl SerializableModule {
    /// Serialize a Module into bytes
    /// The bytes will have the following format:
    /// RKYV serialization (any length) + POS (8 bytes)
    pub fn serialize(&self) -> Result<Vec<u8>, SerializeError> {
        let mut serializer = AllocSerializer::<4096>::default();
        let _pos = serializer
            .serialize_value(self)
            .map_err(to_serialize_error)? as u64;
        let serialized_data = serializer.into_serializer().into_inner();
        //serialized_data.extend_from_slice(&pos.to_le_bytes());
        Ok(serialized_data.to_vec())
    }

    /// Deserialize a Module from a slice.
    /// The slice must have the following format:
    /// RKYV serialization (any length) + POS (8 bytes)
    ///
    /// # Safety
    ///
    /// This method is unsafe since it deserializes data directly
    /// from memory.
    /// Right now we are not doing any extra work for validation, but
    /// `rkyv` has an option to do bytecheck on the serialized data before
    /// serializing (via `rkyv::check_archived_value`).
    pub unsafe fn deserialize(metadata_slice: &[u8]) -> Result<Self, DeserializeError> {
        let archived = Self::archive_from_slice(metadata_slice)?;
        Self::deserialize_from_archive(archived)
    }

    /// # Safety
    ///
    /// This method is unsafe.
    /// Please check `SerializableModule::deserialize` for more details.
    unsafe fn archive_from_slice<'a>(
        buf: &'a [u8],
    ) -> Result<&'a ArchivedSerializableModule, DeserializeError> {
        Ok(rkyv::util::archived_root::<SerializableModule>(buf))
        /*
        if metadata_slice.len() < 8 {
            return Err(DeserializeError::Incompatible(
                "invalid serialized data".into(),
            ));
        }
        let mut pos: [u8; 8] = Default::default();
        pos.copy_from_slice(&metadata_slice[metadata_slice.len() - 8..metadata_slice.len()]);
        let pos: u64 = u64::from_le_bytes(pos);
        Ok(rkyv::archived_root::<Self>(
            &metadata_slice//, //[..metadata_slice.len() - 8],
            //0,
        ))
            */
    }

    /// Deserialize a compilation module from an archive
    pub fn deserialize_from_archive(
        archived: &ArchivedSerializableModule,
    ) -> Result<Self, DeserializeError> {
        let mut deserializer = SharedDeserializeMap::new();
        RkyvDeserialize::deserialize(archived, &mut deserializer)
            .map_err(|e| DeserializeError::CorruptedBinary(format!("{:?}", e)))
    }
}

impl ArchivedSerializableModule {
    /// Zero-copy deserialize from a bytes buffer
    pub unsafe fn from_slice(buf: &[u8]) -> &Self {
        rkyv::util::archived_root::<SerializableModule>(buf)
    }
}
