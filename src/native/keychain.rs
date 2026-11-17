// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, ensure};
use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    data::CFData,
    dictionary::{CFDictionary, CFDictionaryRef},
    string::{CFString, CFStringRef},
};
use std::{path::Path, ptr};

const NOT_FOUND: i32 = -25300;
const DUPLICATE: i32 = -25299;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: CFStringRef;
    static kSecClassGenericPassword: CFStringRef;
    static kSecAttrService: CFStringRef;
    static kSecAttrAccount: CFStringRef;
    static kSecReturnData: CFStringRef;
    static kSecValueData: CFStringRef;
    fn SecItemCopyMatching(query: CFDictionaryRef, result: *mut CFTypeRef) -> i32;
    fn SecItemAdd(attributes: CFDictionaryRef, result: *mut CFTypeRef) -> i32;
    fn SecItemUpdate(query: CFDictionaryRef, attributes: CFDictionaryRef) -> i32;
    fn SecItemDelete(query: CFDictionaryRef) -> i32;
}

type Query = Vec<(CFString, CFType)>;

fn query(directory: &Path) -> Result<Query> {
    let directory = directory.canonicalize().context("无法定位授权数据目录")?;
    let profile = directory.to_str().context("授权数据目录不是 UTF-8 路径")?;
    // Security.framework owns the constant keys. The dictionary retains its values.
    unsafe {
        Ok(vec![
            (
                CFString::wrap_under_get_rule(kSecClass),
                CFString::wrap_under_get_rule(kSecClassGenericPassword).as_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecAttrService),
                CFString::new("io.github.fuck996.QuickADB.host").as_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecAttrAccount),
                CFString::new(profile).as_CFType(),
            ),
        ])
    }
}

pub(super) fn load(directory: &Path) -> Result<Option<Vec<u8>>> {
    let mut query = query(directory)?;
    query.push(unsafe {
        (
            CFString::wrap_under_get_rule(kSecReturnData),
            CFBoolean::true_value().as_CFType(),
        )
    });
    let query = CFDictionary::from_CFType_pairs(&query);
    let mut result = ptr::null();
    let status = unsafe { SecItemCopyMatching(query.as_concrete_TypeRef(), &mut result) };
    if status == NOT_FOUND {
        return Ok(None);
    }
    ensure!(
        status == 0 && !result.is_null(),
        "无法读取钥匙串中的 ADB 授权密钥（OSStatus {status}）"
    );
    let item = unsafe { CFType::wrap_under_create_rule(result) };
    let data = item
        .downcast::<CFData>()
        .context("钥匙串中的 ADB 授权密钥格式无效")?;
    Ok(Some(data.bytes().to_vec()))
}

pub(super) fn save(directory: &Path, bytes: &[u8]) -> Result<()> {
    let mut query = query(directory)?;
    let attributes = unsafe {
        [(
            CFString::wrap_under_get_rule(kSecValueData),
            CFData::from_buffer(bytes).as_CFType(),
        )]
    };
    query.extend_from_slice(&attributes);
    let entry = CFDictionary::from_CFType_pairs(&query);
    let mut status = unsafe { SecItemAdd(entry.as_concrete_TypeRef(), ptr::null_mut()) };
    if status == DUPLICATE {
        query.pop();
        let query = CFDictionary::from_CFType_pairs(&query);
        let attributes = CFDictionary::from_CFType_pairs(&attributes);
        status = unsafe {
            SecItemUpdate(
                query.as_concrete_TypeRef(),
                attributes.as_concrete_TypeRef(),
            )
        };
    }
    ensure!(
        status == 0,
        "无法保存 ADB 授权密钥到钥匙串（OSStatus {status}）"
    );
    Ok(())
}

pub(super) fn delete(directory: &Path) -> Result<()> {
    let query = CFDictionary::from_CFType_pairs(&query(directory)?);
    let status = unsafe { SecItemDelete(query.as_concrete_TypeRef()) };
    ensure!(
        status == 0 || status == NOT_FOUND,
        "无法删除测试钥匙串条目（OSStatus {status}）"
    );
    Ok(())
}
