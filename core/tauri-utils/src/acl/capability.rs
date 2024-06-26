// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! End-user abstraction for selecting permissions a window has access to.

use std::{path::Path, str::FromStr};

use crate::{acl::Identifier, platform::Target};
use serde::{
  de::{Error, IntoDeserializer},
  Deserialize, Deserializer, Serialize,
};
use serde_untagged::UntaggedEnumVisitor;

use super::Scopes;

/// An entry for a permission value in a [`Capability`] can be either a raw permission [`Identifier`]
/// or an object that references a permission and extends its scope.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum PermissionEntry {
  /// Reference a permission or permission set by identifier.
  PermissionRef(Identifier),
  /// Reference a permission or permission set by identifier and extends its scope.
  ExtendedPermission {
    /// Identifier of the permission or permission set.
    identifier: Identifier,
    /// Scope to append to the existing permission scope.
    #[serde(default, flatten)]
    scope: Scopes,
  },
}

impl PermissionEntry {
  /// The identifier of the permission referenced in this entry.
  pub fn identifier(&self) -> &Identifier {
    match self {
      Self::PermissionRef(identifier) => identifier,
      Self::ExtendedPermission {
        identifier,
        scope: _,
      } => identifier,
    }
  }
}

impl<'de> Deserialize<'de> for PermissionEntry {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    #[derive(Deserialize)]
    struct ExtendedPermissionStruct {
      identifier: Identifier,
      #[serde(default, flatten)]
      scope: Scopes,
    }

    UntaggedEnumVisitor::new()
      .string(|string| {
        let de = string.into_deserializer();
        Identifier::deserialize(de).map(Self::PermissionRef)
      })
      .map(|map| {
        let ext_perm = map.deserialize::<ExtendedPermissionStruct>()?;
        Ok(Self::ExtendedPermission {
          identifier: ext_perm.identifier,
          scope: ext_perm.scope,
        })
      })
      .deserialize(deserializer)
  }
}

/// A grouping and boundary mechanism developers can use to separate windows or plugins functionality from each other at runtime.
///
/// If a window is not matching any capability then it has no access to the IPC layer at all.
///
/// This can be done to create trust groups and reduce impact of vulnerabilities in certain plugins or windows.
/// Windows can be added to a capability by exact name or glob patterns like *, admin-* or main-window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Capability {
  /// Identifier of the capability.
  pub identifier: String,
  /// Description of the capability.
  #[serde(default)]
  pub description: String,
  /// Configure remote URLs that can use the capability permissions.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub remote: Option<CapabilityRemote>,
  /// Whether this capability is enabled for local app URLs or not. Defaults to `true`.
  #[serde(default = "default_capability_local")]
  pub local: bool,
  /// List of windows that uses this capability. Can be a glob pattern.
  ///
  /// On multiwebview windows, prefer [`Self::webviews`] for a fine grained access control.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub windows: Vec<String>,
  /// List of webviews that uses this capability. Can be a glob pattern.
  ///
  /// This is only required when using on multiwebview contexts, by default
  /// all child webviews of a window that matches [`Self::windows`] are linked.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub webviews: Vec<String>,
  /// List of permissions attached to this capability. Must include the plugin name as prefix in the form of `${plugin-name}:${permission-name}`.
  pub permissions: Vec<PermissionEntry>,
  /// Target platforms this capability applies. By default all platforms are affected by this capability.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub platforms: Option<Vec<Target>>,
}

fn default_capability_local() -> bool {
  true
}

/// Configuration for remote URLs that are associated with the capability.
#[derive(Debug, Default, Clone, Serialize, Deserialize, Eq, PartialEq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRemote {
  /// Remote domains this capability refers to using the [URLPattern standard](https://urlpattern.spec.whatwg.org/).
  ///
  /// # Examples
  ///
  /// - "https://*.mydomain.dev": allows subdomains of mydomain.dev
  /// - "https://mydomain.dev/api/*": allows any subpath of mydomain.dev/api
  pub urls: Vec<String>,
}

/// Capability formats accepted in a capability file.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", serde(untagged))]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub enum CapabilityFile {
  /// A single capability.
  Capability(Capability),
  /// A list of capabilities.
  List(Vec<Capability>),
  /// A list of capabilities.
  NamedList {
    /// The list of capabilities.
    capabilities: Vec<Capability>,
  },
}

impl CapabilityFile {
  /// Load the given capability file.
  pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, super::Error> {
    let path = path.as_ref();
    let capability_file = std::fs::read_to_string(path).map_err(super::Error::ReadFile)?;
    let ext = path.extension().unwrap().to_string_lossy().to_string();
    let file: Self = match ext.as_str() {
      "toml" => toml::from_str(&capability_file)?,
      "json" => serde_json::from_str(&capability_file)?,
      _ => return Err(super::Error::UnknownCapabilityFormat(ext)),
    };
    Ok(file)
  }
}

impl<'de> Deserialize<'de> for CapabilityFile {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    UntaggedEnumVisitor::new()
      .seq(|seq| seq.deserialize::<Vec<Capability>>().map(Self::List))
      .map(|map| {
        #[derive(Deserialize)]
        struct CapabilityNamedList {
          capabilities: Vec<Capability>,
        }

        let value: serde_json::Map<String, serde_json::Value> = map.deserialize()?;
        if value.contains_key("capabilities") {
          serde_json::from_value::<CapabilityNamedList>(value.into())
            .map(|named| Self::NamedList {
              capabilities: named.capabilities,
            })
            .map_err(|e| serde_untagged::de::Error::custom(e.to_string()))
        } else {
          serde_json::from_value::<Capability>(value.into())
            .map(Self::Capability)
            .map_err(|e| serde_untagged::de::Error::custom(e.to_string()))
        }
      })
      .deserialize(deserializer)
  }
}

impl FromStr for CapabilityFile {
  type Err = super::Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    serde_json::from_str(s)
      .or_else(|_| toml::from_str(s))
      .map_err(Into::into)
  }
}

#[cfg(feature = "build")]
mod build {
  use std::convert::identity;

  use proc_macro2::TokenStream;
  use quote::{quote, ToTokens, TokenStreamExt};

  use super::*;
  use crate::{literal_struct, tokens::*};

  impl ToTokens for CapabilityRemote {
    fn to_tokens(&self, tokens: &mut TokenStream) {
      let urls = vec_lit(&self.urls, str_lit);
      literal_struct!(
        tokens,
        ::tauri::utils::acl::capability::CapabilityRemote,
        urls
      );
    }
  }

  impl ToTokens for PermissionEntry {
    fn to_tokens(&self, tokens: &mut TokenStream) {
      let prefix = quote! { ::tauri::utils::acl::capability::PermissionEntry };

      tokens.append_all(match self {
        Self::PermissionRef(id) => {
          quote! { #prefix::PermissionRef(#id) }
        }
        Self::ExtendedPermission { identifier, scope } => {
          quote! { #prefix::ExtendedPermission {
            identifier: #identifier,
            scope: #scope
          } }
        }
      });
    }
  }

  impl ToTokens for Capability {
    fn to_tokens(&self, tokens: &mut TokenStream) {
      let identifier = str_lit(&self.identifier);
      let description = str_lit(&self.description);
      let remote = opt_lit(self.remote.as_ref());
      let local = self.local;
      let windows = vec_lit(&self.windows, str_lit);
      let webviews = vec_lit(&self.webviews, str_lit);
      let permissions = vec_lit(&self.permissions, identity);
      let platforms = opt_vec_lit(self.platforms.as_ref(), identity);

      literal_struct!(
        tokens,
        ::tauri::utils::acl::capability::Capability,
        identifier,
        description,
        remote,
        local,
        windows,
        webviews,
        permissions,
        platforms
      );
    }
  }
}

#[cfg(test)]
mod tests {
  use crate::acl::{Identifier, Scopes};

  use super::{Capability, CapabilityFile, PermissionEntry};

  #[test]
  fn permission_entry_de() {
    let identifier = Identifier::try_from("plugin:perm".to_string()).unwrap();
    let identifier_json = serde_json::to_string(&identifier).unwrap();
    assert_eq!(
      serde_json::from_str::<PermissionEntry>(&identifier_json).unwrap(),
      PermissionEntry::PermissionRef(identifier.clone())
    );

    assert_eq!(
      serde_json::from_value::<PermissionEntry>(serde_json::json!({
        "identifier": identifier,
        "allow": [],
        "deny": null
      }))
      .unwrap(),
      PermissionEntry::ExtendedPermission {
        identifier,
        scope: Scopes {
          allow: Some(vec![]),
          deny: None
        }
      }
    );
  }

  #[test]
  fn capability_file_de() {
    let capability = Capability {
      identifier: "test".into(),
      description: "".into(),
      remote: None,
      local: true,
      windows: vec![],
      webviews: vec![],
      permissions: vec![],
      platforms: None,
    };
    let capability_json = serde_json::to_string(&capability).unwrap();

    assert_eq!(
      serde_json::from_str::<CapabilityFile>(&capability_json).unwrap(),
      CapabilityFile::Capability(capability.clone())
    );

    assert_eq!(
      serde_json::from_str::<CapabilityFile>(&format!("[{capability_json}]")).unwrap(),
      CapabilityFile::List(vec![capability.clone()])
    );

    assert_eq!(
      serde_json::from_str::<CapabilityFile>(&format!(
        "{{ \"capabilities\": [{capability_json}] }}"
      ))
      .unwrap(),
      CapabilityFile::NamedList {
        capabilities: vec![capability.clone()]
      }
    );
  }
}
