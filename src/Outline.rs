use lopdf::{Document, Object, ObjectId};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub title: String,
    pub page_idx: Option<usize>,
    pub children: Vec<OutlineItem>,
}

fn pdf_doc_encoding_to_unicode() -> [char; 256] {
    let mut table = ['\0'; 256];
    for i in 0..256 {
        table[i] = i as u8 as char;
    }
    table[0x18] = '\u{02D8}';
    table[0x19] = '\u{02C7}';
    table[0x1A] = '\u{02C6}';
    table[0x1B] = '\u{02D9}';
    table[0x1C] = '\u{00A8}';
    table[0x1D] = '\u{02DA}';
    table[0x1E] = '\u{00B8}';
    table[0x1F] = '\u{02DD}';
    table[0x80] = '\u{2022}';
    table[0x81] = '\u{2020}';
    table[0x82] = '\u{2021}';
    table[0x83] = '\u{2026}';
    table[0x84] = '\u{2014}';
    table[0x85] = '\u{2013}';
    table[0x86] = '\u{0192}';
    table[0x87] = '\u{2044}';
    table[0x88] = '\u{2039}';
    table[0x89] = '\u{203A}';
    table[0x8A] = '\u{2212}';
    table[0x8B] = '\u{2030}';
    table[0x8C] = '\u{201E}';
    table[0x8D] = '\u{201C}';
    table[0x8E] = '\u{201D}';
    table[0x8F] = '\u{2018}';
    table[0x90] = '\u{2019}';
    table[0x91] = '\u{201A}';
    table[0x92] = '\u{2122}';
    table[0x93] = '\u{FB01}';
    table[0x94] = '\u{FB02}';
    table[0x95] = '\u{0141}';
    table[0x96] = '\u{0152}';
    table[0x97] = '\u{0160}';
    table[0x98] = '\u{0178}';
    table[0x99] = '\u{017D}';
    table[0x9A] = '\u{0131}';
    table[0x9B] = '\u{0142}';
    table[0x9C] = '\u{0153}';
    table[0x9D] = '\u{0161}';
    table[0x9E] = '\u{017E}';
    table[0x9F] = '\u{FFFD}';
    table[0xA0] = '\u{20AC}';
    table
}

fn decode_pdf_bytes(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let mut u16s = Vec::new();
        for chunk in bytes[2..].chunks(2) {
            if chunk.len() == 2 {
                u16s.push(u16::from_be_bytes([chunk[0], chunk[1]]));
            }
        }
        return String::from_utf16_lossy(&u16s);
    }
    if bytes.len() >= 4 && bytes.len() % 2 == 0 {
        let null_count = bytes.iter().filter(|&&b| b == 0).count();
        if null_count * 3 > bytes.len() || (bytes[0] == 0 && bytes[2] == 0) || (bytes[1] == 0 && bytes[3] == 0) {
            let mut u16s = Vec::new();
            for chunk in bytes.chunks(2) {
                if chunk.len() == 2 {
                    u16s.push(u16::from_be_bytes([chunk[0], chunk[1]]));
                }
            }
            let s = String::from_utf16_lossy(&u16s);
            if s.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t').count() > s.len() / 2 {
                if s.chars().any(|c| c as u32 > 0x7F) || null_count > bytes.len() / 3 {
                    return s;
                }
            }
        }
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let table = pdf_doc_encoding_to_unicode();
    bytes.iter().map(|&b| table[b as usize]).collect()
}

fn decode_title(obj: &Object) -> String {
    match obj {
        Object::String(s, _) => decode_pdf_bytes(s),
        Object::Name(n) => decode_pdf_bytes(n),
        _ => format!("{:?}", obj),
    }
}

fn decode_name_string(obj: &Object) -> Option<String> {
    match obj {
        Object::String(s, _) => Some(decode_pdf_bytes(s)),
        Object::Name(n) => Some(decode_pdf_bytes(n)),
        _ => None,
    }
}

fn build_page_map(doc: &Document) -> HashMap<ObjectId, usize> {
    let pages = doc.get_pages();
    if !pages.is_empty() {
        let mut map = HashMap::new();
        for (page_num, obj_id) in pages {
            map.insert(obj_id, (page_num - 1) as usize);
        }
        return map;
    }
    let mut map = HashMap::new();
    let mut visited = std::collections::HashSet::new();
    let mut counter = 0usize;
    fn walk_pages(
        doc: &Document,
        node_obj: &Object,
        map: &mut HashMap<ObjectId, usize>,
        visited: &mut std::collections::HashSet<ObjectId>,
        counter: &mut usize,
        depth: usize,
    ) {
        if depth > 128 { return; }
        let node_ref = match node_obj {
            Object::Reference(id) => {
                if !visited.insert(*id) { return; }
                match doc.get_object(*id) {
                    Ok(o) => o,
                    Err(_) => return,
                }
            }
            _ => node_obj,
        };
        let dict = match node_ref {
            Object::Dictionary(d) => d,
            _ => return,
        };
        let type_name = dict.get(b"Type").ok().and_then(|o| {
            match o {
                Object::Name(n) => Some(String::from_utf8_lossy(n).to_string()),
                _ => None,
            }
        });
        if let Some(t) = type_name {
            if t == "Page" {
                if let Object::Reference(id) = node_obj {
                    map.insert(*id, *counter);
                }
                *counter += 1;
                return;
            }
        }
        if let Ok(kids_obj) = dict.get(b"Kids") {
            let kids_resolved = if let Object::Reference(kid_id) = kids_obj {
                if let Ok(k) = doc.get_object(*kid_id) { k } else { return; }
            } else {
                kids_obj
            };
            if let Object::Array(kids_arr) = kids_resolved {
                for kid in kids_arr {
                    walk_pages(doc, kid, map, visited, counter, depth + 1);
                }
            }
        }
    }
    if let Ok(Object::Reference(root_id)) = doc.trailer.get(b"Root") {
        if let Ok(Object::Dictionary(root_dict)) = doc.get_object(*root_id) {
            if let Ok(pages_obj) = root_dict.get(b"Pages") {
                walk_pages(doc, pages_obj, &mut map, &mut visited, &mut counter, 0);
            }
        }
    }
    if map.is_empty() {
        for (oid, obj) in &doc.objects {
            if let Object::Dictionary(d) = obj {
                if let Ok(Object::Name(t)) = d.get(b"Type") {
                    if String::from_utf8_lossy(t).to_string() == "Page" {
                        if !map.contains_key(oid) {
                            map.insert(*oid, counter);
                            counter += 1;
                        }
                    }
                }
            }
        }
    }
    map
}

fn find_page_by_id(page_map: &HashMap<ObjectId, usize>, target: ObjectId) -> Option<usize> {
    if let Some(idx) = page_map.get(&target) {
        return Some(*idx);
    }
    for (obj_id, idx) in page_map {
        if obj_id.0 == target.0 {
            return Some(*idx);
        }
    }
    None
}

fn deref_obj<'a>(doc: &'a Document, obj: &'a Object) -> &'a Object {
    if let Object::Reference(id) = obj {
        if let Ok(resolved) = doc.get_object(*id) {
            return resolved;
        }
    }
    obj
}

fn lookup_name_tree_impl(
    doc: &Document,
    node: &Object,
    name: &str,
    depth: usize,
    visited: &mut std::collections::HashSet<ObjectId>,
) -> Option<Object> {
    if depth > 32 { return None; }
    if let Object::Reference(id) = node {
        if !visited.insert(*id) { return None; }
    }
    let node = deref_obj(doc, node);
    let dict = match node {
        Object::Dictionary(d) => d,
        _ => return None,
    };
    if let Ok(names) = dict.get(b"Names") {
        if let Object::Array(arr) = deref_obj(doc, names) {
            for pair in arr.chunks(2) {
                if pair.len() == 2 {
                    if let Some(n) = decode_name_string(deref_obj(doc, &pair[0])) {
                        if n == name {
                            return Some(pair[1].clone());
                        }
                    }
                }
            }
        }
    }
    if let Ok(kids) = dict.get(b"Kids") {
        if let Object::Array(kids_arr) = deref_obj(doc, kids) {
            for kid in kids_arr {
                if let Some(found) = lookup_name_tree_impl(doc, kid, name, depth + 1, visited) {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn lookup_name_tree(doc: &Document, node: &Object, name: &str) -> Option<Object> {
    let mut visited = std::collections::HashSet::new();
    lookup_name_tree_impl(doc, node, name, 0, &mut visited)
}

fn resolve_named_dest(doc: &Document, name: &str) -> Option<Object> {
    let catalog_id = match doc.trailer.get(b"Root") {
        Ok(Object::Reference(id)) => *id,
        _ => return None,
    };
    let catalog_obj = doc.get_object(catalog_id).ok()?;
    let catalog_dict = match catalog_obj {
        Object::Dictionary(d) => d,
        _ => return None,
    };
    if let Ok(dests_obj) = catalog_dict.get(b"Dests") {
        let dests_dict_obj = deref_obj(doc, dests_obj);
        if let Object::Dictionary(dests_dict) = dests_dict_obj {
            if let Ok(v) = dests_dict.get(name.as_bytes()) {
                return Some(v.clone());
            }
            for (k, v) in dests_dict.iter() {
                if String::from_utf8_lossy(k).to_string() == name {
                    return Some(v.clone());
                }
            }
        }
    }
    if let Ok(names_obj) = catalog_dict.get(b"Names") {
        let names_dict_obj = deref_obj(doc, names_obj);
        if let Object::Dictionary(names_dict) = names_dict_obj {
            if let Ok(dests_obj) = names_dict.get(b"Dests") {
                if let Some(found) = lookup_name_tree(doc, dests_obj, name) {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn resolve_dest_to_page_recursive(
    doc: &Document,
    dest_obj: &Object,
    page_map: &HashMap<ObjectId, usize>,
    depth: usize,
) -> Option<usize> {
    if depth > 16 { return None; }
    if let Object::Reference(id) = dest_obj {
        if let Some(page_idx) = find_page_by_id(page_map, *id) {
            return Some(page_idx);
        }
        if let Ok(resolved) = doc.get_object(*id) {
            return resolve_dest_to_page_recursive(doc, resolved, page_map, depth + 1);
        }
        return None;
    }
    let obj = deref_obj(doc, dest_obj);
    match obj {
        Object::Array(arr) => {
            if arr.is_empty() { return None; }
            match &arr[0] {
                Object::Reference(id) => find_page_by_id(page_map, *id),
                Object::Dictionary(dict) => {
                    if let Ok(Object::Name(t)) = dict.get(b"Type") {
                        if String::from_utf8_lossy(t).to_string() == "Page" {
                            for (oid, o) in &doc.objects {
                                if let Object::Dictionary(d) = o {
                                    if d.len() == dict.len() {
                                        if let (Ok(a), Ok(b)) = (d.get(b"MediaBox"), dict.get(b"MediaBox")) {
                                            if format!("{:?}", a) == format!("{:?}", b) {
                                                if let Some(idx) = find_page_by_id(page_map, *oid) {
                                                    return Some(idx);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if !page_map.is_empty() { return Some(0); }
                        }
                    }
                    None
                }
                Object::Integer(n) => {
                    let n = *n as usize;
                    if n < page_map.len() { Some(n) } else { Some(n.saturating_sub(1).min(page_map.len().saturating_sub(1))) }
                }
                _ => None,
            }
        }
        Object::String(_, _) | Object::Name(_) => {
            if let Some(name) = decode_name_string(obj) {
                if let Some(named_dest) = resolve_named_dest(doc, &name) {
                    return resolve_dest_to_page_recursive(doc, &named_dest, page_map, depth + 1);
                }
            }
            None
        }
        Object::Dictionary(dict) => {
            if let Ok(d) = dict.get(b"D") {
                return resolve_dest_to_page_recursive(doc, d, page_map, depth + 1);
            }
            None
        }
        _ => None,
    }
}

fn get_outline_item_title_and_dest(
    doc: &Document,
    obj_id: ObjectId,
    page_map: &HashMap<ObjectId, usize>,
) -> Option<(String, Option<usize>, Option<ObjectId>, Option<ObjectId>)> {
    let obj = doc.get_object(obj_id).ok()?;
    let dict = match obj {
        Object::Dictionary(d) => d,
        _ => return None,
    };
    let title = dict.get(b"Title").ok().map(decode_title).unwrap_or_else(|| "Untitled".to_string());
    let mut page_idx: Option<usize> = None;
    if let Ok(dest) = dict.get(b"Dest") {
        page_idx = resolve_dest_to_page_recursive(doc, dest, page_map, 0);
    }
    if page_idx.is_none() {
        if let Ok(a_obj) = dict.get(b"A") {
            let a_resolved = deref_obj(doc, a_obj);
            if let Object::Dictionary(a_dict) = a_resolved {
                let is_goto = if let Ok(s_obj) = a_dict.get(b"S") {
                    if let Some(s_name) = decode_name_string(deref_obj(doc, s_obj)) {
                        s_name == "GoTo" || s_name == "GoToR"
                    } else { true }
                } else { true };
                if is_goto {
                    if let Ok(d) = a_dict.get(b"D") {
                        page_idx = resolve_dest_to_page_recursive(doc, d, page_map, 0);
                    }
                }
            }
        }
    }
    let first = dict.get(b"First").ok().and_then(|o| match o { Object::Reference(id) => Some(*id), _ => None });
    let next = dict.get(b"Next").ok().and_then(|o| match o { Object::Reference(id) => Some(*id), _ => None });
    Some((title, page_idx, first, next))
}

fn collect_siblings(
    doc: &Document,
    start_id: Option<ObjectId>,
    page_map: &HashMap<ObjectId, usize>,
) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    let mut current = start_id;
    let mut visited = std::collections::HashSet::new();
    while let Some(id) = current {
        if !visited.insert(id) { break; }
        if let Some((title, page_opt, first_child, next_id)) = get_outline_item_title_and_dest(doc, id, page_map) {
            let children = if let Some(first) = first_child { collect_siblings(doc, Some(first), page_map) } else { Vec::new() };
            items.push(OutlineItem { title, page_idx: page_opt, children });
            current = next_id;
        } else { break; }
    }
    items
}

pub fn extract_outline(bytes: &[u8]) -> Vec<OutlineItem> {
    let doc = match Document::load_mem(bytes) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let page_map = build_page_map(&doc);
    let catalog_id = match doc.trailer.get(b"Root") {
        Ok(Object::Reference(id)) => *id,
        _ => return Vec::new(),
    };
    let catalog_obj = match doc.get_object(catalog_id) {
        Ok(o) => o,
        _ => return Vec::new(),
    };
    let catalog_dict = match deref_obj(&doc, catalog_obj) {
        Object::Dictionary(d) => d,
        _ => return Vec::new(),
    };
    let outlines_obj = match catalog_dict.get(b"Outlines") {
        Ok(o) => deref_obj(&doc, o),
        _ => return Vec::new(),
    };
    let outlines_dict = match outlines_obj {
        Object::Dictionary(d) => d,
        _ => return Vec::new(),
    };
    let first_id = match outlines_dict.get(b"First") {
        Ok(Object::Reference(id)) => Some(*id),
        _ => None,
    };
    if let Some(first) = first_id {
        collect_siblings(&doc, Some(first), &page_map)
    } else {
        Vec::new()
    }
}
