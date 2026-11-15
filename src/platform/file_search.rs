//! Read-only access to indexed Windows files. Run on an owned worker thread.
use std::{
    mem::ManuallyDrop,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, CLSCTX_NO_CODE_DOWNLOAD, COINIT_APARTMENTTHREADED,
                COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoUninitialize,
                DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT,
                DISPPARAMS, EXCEPINFO, IDispatch,
            },
            Ole::DISPID_PROPERTYPUT,
            Variant::VARIANT,
        },
    },
    core::{BSTR, GUID, HRESULT, PCWSTR, w},
};

// ADODB.Connection in the Windows SDK's msado15.idl.
const ADO_CONNECTION: GUID = GUID::from_u128(0x00000514_0000_0010_8000_00aa006d2ea4);
const CONNECTION_STRING: &str =
    "Provider=Search.CollatorDSO;Extended Properties='Application=Windows';";
const UNAVAILABLE: &str =
    "Windows Search is unavailable. Check Search indexing in Windows Settings.";
const QUERY_FAILED: &str = "Couldn’t search indexed files. Try again.";
const QUERY_TIMEOUT: &str = "File search timed out. Try a more specific name.";
const MAX_RESULTS: usize = 50;
const MAX_NAME_UNITS: usize = 255;

#[derive(Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub is_folder: bool,
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: this guard is kept on the initialized thread until its COM values drop.
        unsafe { CoUninitialize() };
    }
}

/// Reads filesystem names from the existing Windows Search catalog. ADO returns
/// a read-only, forward-only rowset, so no index or file changes are made here.
pub fn search(query: &str, limit: usize) -> Result<Vec<SearchMatch>, String> {
    let limit = limit.min(MAX_RESULTS);
    let Some((exact, partial)) = sql_queries(query, limit) else {
        return Ok(Vec::new());
    };

    // SAFETY: this function runs on a worker. COM objects are dropped before
    // uninitialization, and only owned paths and flags leave the thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
        .ok()
        .map_err(|_| UNAVAILABLE)?;
    let _apartment = Apartment;
    let connection: IDispatch = unsafe {
        CoCreateInstance(
            &ADO_CONNECTION,
            None,
            CLSCTX_INPROC_SERVER | CLSCTX_NO_CODE_DOWNLOAD,
        )
    }
    .map_err(|_| UNAVAILABLE)?;
    for (property, timeout) in [(w!("ConnectionTimeout"), 2), (w!("CommandTimeout"), 6)] {
        member(
            &connection,
            property,
            DISPATCH_PROPERTYPUT,
            Some(VARIANT::from(timeout)),
        )
        .map_err(|_| UNAVAILABLE)?;
    }
    member(
        &connection,
        w!("Open"),
        DISPATCH_METHOD,
        Some(CONNECTION_STRING.into()),
    )
    .map_err(|_| UNAVAILABLE)?;

    let result = read_matches(&connection, &exact, limit)
        .and_then(|matches| {
            if matches.is_empty() {
                let query = query.trim().to_lowercase();
                let mut candidates = read_matches(&connection, &partial, limit * 2)?;
                candidates.sort_by_cached_key(|entry| {
                    let name = entry
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase();
                    (
                        !name.starts_with(&query),
                        name,
                        entry.path.to_string_lossy().to_lowercase(),
                        entry.path.clone(),
                    )
                });
                candidates.truncate(limit);
                Ok(candidates)
            } else {
                Ok(matches)
            }
        })
        .map_err(|error| {
            if is_timeout(error.code()) {
                QUERY_TIMEOUT.into()
            } else {
                QUERY_FAILED.into()
            }
        });
    let _ = member(&connection, w!("Close"), DISPATCH_METHOD, None);
    result
}

fn read_matches(
    connection: &IDispatch,
    sql: &str,
    limit: usize,
) -> windows::core::Result<Vec<SearchMatch>> {
    let value = member(connection, w!("Execute"), DISPATCH_METHOD, Some(sql.into()))?;
    let rows = IDispatch::try_from(&value)?;
    let fields = IDispatch::try_from(&member(&rows, w!("Fields"), DISPATCH_PROPERTYGET, None)?)?;
    let field = |index: i32| {
        let value = member(
            &fields,
            w!("Item"),
            DISPATCH_PROPERTYGET,
            Some(index.into()),
        )?;
        IDispatch::try_from(&value)
    };
    let url = field(0)?;
    let attributes = field(1)?;
    let filename = field(2)?;
    let display_path = field(3)?;
    let mut matches = Vec::with_capacity(limit);
    let read: windows::core::Result<()> = (|| {
        while matches.len() < limit
            && !bool::try_from(&member(&rows, w!("EOF"), DISPATCH_PROPERTYGET, None)?)?
        {
            let item_url = BSTR::try_from(&member(&url, w!("Value"), DISPATCH_PROPERTYGET, None)?);
            let item_attributes = u32::try_from(&member(
                &attributes,
                w!("Value"),
                DISPATCH_PROPERTYGET,
                None,
            )?);
            let item_name =
                BSTR::try_from(&member(&filename, w!("Value"), DISPATCH_PROPERTYGET, None)?);
            let display = member(&display_path, w!("Value"), DISPATCH_PROPERTYGET, None)
                .ok()
                .and_then(|value| BSTR::try_from(&value).ok())
                .and_then(|value| String::from_utf16(&value).ok());
            if let (Ok(item_url), Ok(item_attributes), Ok(item_name)) =
                (item_url, item_attributes, item_name)
                && let Ok(item_url) = String::from_utf16(&item_url)
                && let Ok(item_name) = String::from_utf16(&item_name)
                && let Some(path) = file_url_path(&item_url, &item_name, display.as_deref())
            {
                matches.push(SearchMatch {
                    path,
                    is_folder: item_attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
                });
            }
            if matches.len() < limit {
                member(&rows, w!("MoveNext"), DISPATCH_METHOD, None)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = read
        && (matches.is_empty() || !is_timeout(error.code()))
    {
        return Err(error);
    }
    Ok(matches)
}

fn is_timeout(code: HRESULT) -> bool {
    matches!(code.0 as u32, 0x8004_1607 | 0x8004_0e31)
}

fn member(
    object: &IDispatch,
    name: PCWSTR,
    flags: DISPATCH_FLAGS,
    mut argument: Option<VARIANT>,
) -> windows::core::Result<VARIANT> {
    let mut id = 0;
    let mut property_put = DISPID_PROPERTYPUT;
    let mut args = DISPPARAMS::default();
    if let Some(argument) = argument.as_mut() {
        args.rgvarg = argument;
        args.cArgs = 1;
    }
    if flags == DISPATCH_PROPERTYPUT {
        args.rgdispidNamedArgs = &mut property_put;
        args.cNamedArgs = 1;
    }
    let mut value = VARIANT::default();
    let mut exception = EXCEPINFO::default();
    // SAFETY: callers supply a static NUL-terminated name and initialized
    // variants. Every DISPPARAMS pointer remains live through Invoke and COM
    // hands ownership of any exception strings to us.
    let invoked = unsafe {
        let iid_null = GUID::zeroed();
        object.GetIDsOfNames(&iid_null, &name, 1, 0, &mut id)?;
        let result = object.Invoke(
            id,
            &iid_null,
            0,
            flags,
            &args,
            Some(&mut value),
            Some(&mut exception),
            None,
        );
        if let Some(fill) = exception.pfnDeferredFillIn {
            let _ = fill(&mut exception);
        }
        result
    };
    drop(ManuallyDrop::into_inner(exception.bstrSource));
    drop(ManuallyDrop::into_inner(exception.bstrDescription));
    drop(ManuallyDrop::into_inner(exception.bstrHelpFile));
    if invoked.is_err() && exception.scode < 0 {
        return Err(windows::core::Error::from_hresult(HRESULT(exception.scode)));
    }
    invoked?;
    Ok(value)
}

fn file_url_path(item_url: &str, filename: &str, display_path: Option<&str>) -> Option<PathBuf> {
    let (scheme, raw_path) = item_url.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("file") {
        return None;
    }
    let filename = filename.to_lowercase();
    let matches_filename = |path: &PathBuf| {
        super::launcher::is_absolute_folder_path(path)
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_lowercase() == filename)
    };

    // Some index paths have a literal file: wrapper around a Windows path.
    // A second column must corroborate it before interpreting raw # or % as
    // part of the path. The display column alone can be localized.
    if let Some(display) = display_path {
        let raw_path = if let Some(local_path) = raw_path.strip_prefix("///") {
            local_path
        } else if raw_path.starts_with('/') && !raw_path.starts_with("//") {
            &raw_path[1..]
        } else {
            raw_path
        };
        let raw = PathBuf::from(raw_path.replace('/', "\\"));
        if raw == Path::new(display) && matches_filename(&raw) {
            return Some(raw);
        }
    }

    let url = url::Url::parse(item_url).ok()?;
    if url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    let path = url.to_file_path().ok()?;
    matches_filename(&path).then_some(path)
}

fn sql_queries(query: &str, limit: usize) -> Option<(String, String)> {
    let query = query.trim();
    let limit = limit.min(MAX_RESULTS);
    if query.is_empty()
        || limit == 0
        || query.encode_utf16().count() > MAX_NAME_UNITS
        || query.chars().any(char::is_control)
    {
        return None;
    }
    let exact = query.replace('\'', "''");
    let mut pattern = String::with_capacity(exact.len());
    // Square brackets quote LIKE wildcards. A closing bracket is an ordinary
    // character outside a bracketed expression, including after a quoted '['.
    for character in exact.chars() {
        match character {
            '%' => pattern.push_str("[%]"),
            '_' => pattern.push_str("[_]"),
            '[' => pattern.push_str("[[]"),
            _ => pattern.push(character),
        }
    }
    let row_limit = limit * 2;
    let select = format!(
        "SELECT TOP {row_limit} System.ItemUrl, System.FileAttributes, System.FileName, System.ItemPathDisplay \
         FROM SystemIndex WHERE SCOPE='file:' AND System.FileName"
    );
    let order = "ORDER BY System.DateModified DESC, System.FileName ASC, System.ItemUrl ASC";
    // Sort the bounded LIKE candidates locally. Asking the catalog to sort
    // every partial match can make an ordinary miss exceed its query budget.
    // A leading wildcard on a one-letter query matches too much of the catalog.
    let leading = if query.chars().count() > 1 { "%" } else { "" };
    Some((
        format!("{select} = '{exact}' {order}"),
        format!("{select} LIKE '{leading}{pattern}%'"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_queries_are_literal_and_scoped_to_the_filesystem() {
        let (exact, partial) = sql_queries("O'Brien_100%[x].pdf", 6).unwrap();
        assert!(exact.contains("TOP 12 System.ItemUrl"));
        assert!(exact.contains("WHERE SCOPE='file:' AND"));
        assert!(exact.contains("System.FileName = 'O''Brien_100%[x].pdf'"));
        assert!(partial.contains("LIKE '%O''Brien[_]100[%][[]x].pdf%'"));
        assert!(exact.contains("ORDER BY System.DateModified DESC"));
        assert!(!partial.contains("ORDER BY"));
    }

    #[test]
    fn empty_and_unbounded_searches_are_avoided() {
        assert!(sql_queries("   ", 6).is_none());
        assert!(sql_queries("text", 0).is_none());
        assert!(sql_queries("unsafe\0text", 6).is_none());
        assert!(sql_queries(&"a".repeat(256), 6).is_none());
        let (_, partial) = sql_queries("A", usize::MAX).unwrap();
        assert!(partial.contains("SELECT TOP 100 "));
        assert!(partial.contains("LIKE 'A%'"));
        assert!(!partial.contains("LIKE '%A%'"));
    }

    #[test]
    fn file_urls_preserve_exact_unicode_names_and_reject_other_schemes() {
        assert_eq!(
            file_url_path(
                "file:///C:/Reports/%E5%A0%B1%E5%91%8A%20%231%20%25.pdf",
                "報告 #1 %.pdf",
                Some(r"C:\Reports\報告 #1 %.pdf"),
            ),
            Some(PathBuf::from(r"C:\Reports\報告 #1 %.pdf"))
        );
        let raw_path = r"C:\Reports %23\report #2[final].pdf";
        assert_eq!(
            file_url_path(
                "file:C:/Reports %23/report #2[final].pdf",
                "report #2[final].pdf",
                Some(raw_path),
            ),
            Some(PathBuf::from(raw_path))
        );
        assert!(file_url_path("file:///C:/Reports/report#2.pdf", "report#2.pdf", None).is_none());
        assert!(file_url_path("mapi://server/file", "file", None).is_none());
        assert!(file_url_path("file:///C:/Reports/other.txt", "report.txt", None).is_none());
    }
}
