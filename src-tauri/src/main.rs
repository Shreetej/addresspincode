#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use calamine::{open_workbook, Reader, Xlsx, Data, DataType};
use rust_xlsxwriter::{Workbook, XlsxError};
use itertools::Itertools;
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json; 
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::path::PathBuf;
use std::fs::File;
use std::io::BufReader;
use tokio::sync::RwLock;
use lru::LruCache;
use tauri::{State, Emitter, Manager, AppHandle};
use rusqlite::{params, Connection};
use chrono::Utc;
use uuid::Uuid;

// -------- CONFIG & CONSTANTS --------
const CSV_FILENAME: &str = "pincode-chinna.csv"; 
const TOP_K_DEFAULT: usize = 5;
const MIN_LOCALITY_TOKEN_LEN: usize = 3;
const MAX_CANDIDATES: usize = 800;
const MAX_CAP: f64 = 120.0;

const PHRASE_BONUS: f64 = 30.0;
const ADDRESS_TOKEN_BONUS: f64 = 10.0;
const OFFICE_NAME_BONUS: f64 = 20.0;
const POST_OFFICE_BONUS: f64 = 30.0;

// -------- DATA MODELS --------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PincodeRecord {
    pincode: String,
    office_name: String,
    district: String,
    state: String,
    taluk: Option<String>,
    city: Option<String>,
    village: Option<String>,
    office_name_norm_lc: String,
    address_lines_norm_lc: String, 
    full_address_norm_lc: String,
    city_norm_lc: String,
    village_norm_lc: String,
    state_lc: String,
    district_lc: String,
    office_tokens: HashSet<String>,
    address_tokens: HashSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StatePinInfo {
    p1: Vec<String>,
    p2: Vec<String>,
    p3: Vec<String>,
}

#[derive(Clone, Deserialize)]
struct VillageMeta {
    state: Option<String>,
    district: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BulkJob {
    id: String,
    filename: String,
    input_path: String,
    output_path: String,
    timestamp: String,
    status: String,
    total_rows: i64,
    duration_str: String,
}

struct AppState {
    index: Vec<PincodeRecord>,
    inverted_index: HashMap<String, Arc<Vec<usize>>>,
    states: Vec<String>,
    districts: Vec<String>,
    state_pin: HashMap<String, StatePinInfo>,
    district_pin: HashMap<String, Vec<String>>,
    city_pin: HashMap<String, Vec<String>>,
    village_meta: HashMap<String, VillageMeta>,
    village_names: HashMap<String, Vec<usize>>,
    city_names: HashMap<String, Vec<usize>>,
    village_list: Vec<String>,
    city_list: Vec<String>,
    cache: Arc<RwLock<LruCache<String, Vec<(f64, usize)>>>>,
    db_conn: Arc<Mutex<Connection>>,
}

#[derive(Serialize)]
struct PincodeCandidateOut {
    pincode: String,
    office_name: String,
    city: Option<String>,
    district: String,
    state: String,
    score: f64,
}

#[derive(Serialize)]
struct PincodeResolutionOut {
    best_pincode: Option<String>,
    confidence: String,
    candidates: Vec<PincodeCandidateOut>,
    duration_ms: u64,
}

#[derive(Serialize, Clone)]
struct ProgressEvent {
    percent: f64,
    processed: usize,
    total: usize,
    eta_seconds: u64,
}

// -------- DATABASE INIT --------

fn init_db() -> Result<Connection> {
    let db_path = "history.db";
    let conn = Connection::open(db_path)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS bulk_jobs (
            id TEXT PRIMARY KEY,
            filename TEXT NOT NULL,
            input_path TEXT NOT NULL,
            output_path TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            status TEXT NOT NULL,
            total_rows INTEGER DEFAULT 0,
            duration_str TEXT
        )",
        [],
    )?;
    Ok(conn)
}

// -------- NORMALIZATION --------

lazy_static::lazy_static! {
    static ref ABBREV_MAP: Vec<(Regex, &'static str)> = vec![
        (Regex::new(r"\brd\b").unwrap(), " road"),
        (Regex::new(r"\bst\b").unwrap(), " street"),
        (Regex::new(r"\bstr\b").unwrap(), " street"),
        (Regex::new(r"\bcol\b").unwrap(), " colony"),
        (Regex::new(r"\bngr\b").unwrap(), " nagar"),
        (Regex::new(r"\bnagar\b").unwrap(), " nagar"),
        (Regex::new(r"\bdist\b\.?").unwrap(), " district"),
        (Regex::new(r"\bpo\b").unwrap(), " post office"),
        (Regex::new(r"\bps\b").unwrap(), " police station"),
    ];
    static ref CLEAN_REGEX: Regex = Regex::new(r"[^a-z0-9\s]").unwrap();
    static ref SPACE_REGEX: Regex = Regex::new(r"\s+").unwrap();
    // PERFORMANCE FIX: Moved to static to prevent re-compilation 800k times
    static ref PINCODE_REGEX: Regex = Regex::new(r"\b\d{6}\b").unwrap();
}

fn normalize_text(text: &str) -> String {
    if text.trim().is_empty() { return String::new(); }
    let mut s = text.to_lowercase();
    s = s.replace('/', " ").replace('-', " ");
    for (pat, repl) in ABBREV_MAP.iter() {
        s = pat.replace_all(&s, *repl).to_string();
    }
    s = CLEAN_REGEX.replace_all(&s, " ").to_string();
    s = SPACE_REGEX.replace_all(&s, " ").trim().to_string();
    s
}

// -------- INDEXING & LOADING --------

fn tokenize_for_index(s: &str) -> Vec<String> {
    s.split_whitespace()
        .filter(|t| t.len() >= 3 && !t.chars().all(char::is_numeric))
        .map(|t| t.to_string())
        .collect()
}

fn load_json_file<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Option<T> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader).ok()
}

fn load_data(resource_path: &std::path::Path, app_handle: &AppHandle) -> anyhow::Result<AppState> {
    let csv_path = resource_path.join(CSV_FILENAME);
    let final_path = if csv_path.exists() { csv_path } else { PathBuf::from(CSV_FILENAME) };

    if !final_path.exists() {
        return Err(anyhow::anyhow!("CSV file not found at {:?}", final_path));
    }

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(&final_path)?;

    let mut index = Vec::new();
    let mut inverted = HashMap::new();
    let mut states_set = HashSet::new();
    let mut districts_set = HashSet::new();
    let mut count = 0;
    
    for result in reader.deserialize::<HashMap<String, String>>() {
        count += 1;
        if count % 5000 == 0 {
             let _ = app_handle.emit("loading-progress", count);
        }

        let record = match result {
            Ok(r) => r,
            Err(_) => continue,
        };

        let get = |keys: &[&str]| -> String {
            for k in keys {
                if let Some(v) = record.get(*k) {
                    if !v.trim().is_empty() { return v.trim().to_string(); }
                }
            }
            String::new()
        };

        let pincode = get(&["pincode", "PINCODE", "Pincode"]);
        let office_name = get(&["office_name", "OFFICE_NAME"]);
        if pincode.is_empty() || office_name.is_empty() { continue; }
        
        let district = get(&["District", "district"]);
        let state = get(&["state_name", "state"]);
        let taluk = get(&["taluk_name", "taluk"]).into_option();
        let city = get(&["city_name", "city"]).into_option();
        let village = get(&["village_name", "village"]).into_option();

        let office_name_norm = normalize_text(&office_name);
        let address_lines_raw = format!("{} {} {} {} {}", 
            get(&["office_address1"]), get(&["office_address2"]), 
            get(&["office_address3"]), get(&["office_street"]), get(&["landmark"]));
        let address_lines_norm = normalize_text(&address_lines_raw);
        
        let full_raw = format!("{}, {}, {}, {}, {}, {}, {}, {}", 
            office_name, address_lines_raw, village.as_deref().unwrap_or(""), 
            city.as_deref().unwrap_or(""), taluk.as_deref().unwrap_or(""), 
            district, state, pincode);
        let full_address_norm = normalize_text(&full_raw);
        
        let city_norm = city.as_ref().map(|c| normalize_text(c)).unwrap_or_default();
        let village_norm = village.as_ref().map(|v| normalize_text(v)).unwrap_or_default();

        let office_tokens: HashSet<String> = office_name_norm.split_whitespace().map(String::from).collect();
        let address_tokens: HashSet<String> = address_lines_norm.split_whitespace().map(String::from).collect();

        let idx_tokens = tokenize_for_index(&full_address_norm);
        let current_idx = index.len();
        for t in idx_tokens {
            inverted.entry(t).or_insert_with(Vec::new).push(current_idx);
        }

        states_set.insert(state.to_lowercase());
        districts_set.insert(district.to_lowercase());

        index.push(PincodeRecord {
            pincode, office_name, district: district.clone(), state: state.clone(),
            taluk, city, village,
            office_name_norm_lc: office_name_norm.to_lowercase(),
            address_lines_norm_lc: address_lines_norm.to_lowercase(), 
            full_address_norm_lc: full_address_norm.to_lowercase(),
            city_norm_lc: city_norm.to_lowercase(),
            village_norm_lc: village_norm.to_lowercase(),
            state_lc: state.to_lowercase(),
            district_lc: district.to_lowercase(),
            office_tokens, address_tokens,
        });
    }

    for list in inverted.values_mut() {
        list.sort();
        list.dedup();
        list.shrink_to_fit();
    }
    
    let inverted_index: HashMap<String, Arc<Vec<usize>>> = inverted.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
    
    let mut states: Vec<String> = states_set.into_iter().collect();
    states.sort_by(|a, b| b.len().cmp(&a.len()));
    
    let mut districts: Vec<String> = districts_set.into_iter().collect();
    districts.sort_by(|a, b| b.len().cmp(&a.len()));

    // JSON Loading matching API
    let json_dir = resource_path.join("json");
    let state_pin = load_json_file::<HashMap<String, StatePinInfo>>(&json_dir.join("state_pin.json")).unwrap_or_default();
    let district_pin = load_json_file::<HashMap<String, Vec<String>>>(&json_dir.join("district_pin.json")).unwrap_or_default();
    let city_pin = load_json_file::<HashMap<String, Vec<String>>>(&json_dir.join("city_pin.json")).unwrap_or_default();
    let village_meta = HashMap::new(); 

    let mut village_names = HashMap::new();
    let mut city_names = HashMap::new();
    for (i, rec) in index.iter().enumerate() {
        if !rec.village_norm_lc.is_empty() {
            village_names.entry(rec.village_norm_lc.clone()).or_insert_with(Vec::new).push(i);
        }
        if !rec.city_norm_lc.is_empty() {
            city_names.entry(rec.city_norm_lc.clone()).or_insert_with(Vec::new).push(i);
        }
    }
    let village_list: Vec<String> = village_names.keys().cloned().collect();
    let city_list: Vec<String> = city_names.keys().cloned().collect();

    println!("Loaded {} records.", index.len());
    let db_conn = init_db()?;

    Ok(AppState {
        index, inverted_index,
        states, districts,
        state_pin, district_pin, city_pin,
        village_meta, village_names, city_names,
        village_list, city_list,
        cache: Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(20_000).unwrap()))),
        db_conn: Arc::new(Mutex::new(db_conn)),
    })
}

trait EmptyOptionStr { fn into_option(self) -> Option<String>; }
impl EmptyOptionStr for String { fn into_option(self) -> Option<String> { if self.is_empty() { None } else { Some(self) } } }

// -------- SEARCH LOGIC --------

fn extract_locality_phrases(norm_query: &str) -> Vec<String> {
    let tokens: Vec<&str> = norm_query.split_whitespace().collect();
    let mut phrases = Vec::new();
    let max_tail = 4;

    let tail_len = tokens.len().min(max_tail);
    let start_idx = tokens.len() - tail_len;
    let tail = &tokens[start_idx..];

    for n in (1..=tail.len()).rev() {
        let phrase = tail[tail.len() - n..].join(" ");
        if phrase.len() >= MIN_LOCALITY_TOKEN_LEN {
            phrases.push(phrase);
        }
    }

    let post_patterns = ["post", "postoffice", "s.o", "so", "po", "p.o"];
    for (i, tok) in tokens.iter().enumerate() {
        if post_patterns.contains(tok) && i + 1 < tokens.len() {
            let end = (i + 1 + 3).min(tokens.len());
            for n in 1..=(end - (i + 1)) {
                let phrase = tokens[i + 1..i + 1 + n].join(" ");
                if phrase.len() >= MIN_LOCALITY_TOKEN_LEN {
                    phrases.insert(0, phrase);
                }
            }
        }
    }

    phrases.into_iter().unique().collect()
}

fn compute_score(
    rec: &PincodeRecord, 
    query: &str, 
    query_tokens: &[String], 
    locality_phrases: &[String]
) -> f64 {
    let mut score = 0.0;
    let query_lower = query.to_lowercase();
    
    for token in query_tokens {
        if token.len() >= 3 && rec.office_tokens.contains(token) {
            score += OFFICE_NAME_BONUS;
        }
    }
    
    if query_lower.contains("post office") && rec.office_name_norm_lc.contains("post office") {
        score += POST_OFFICE_BONUS;
    }
    
    for token in query_tokens {
        if token.len() >= 3 && rec.address_tokens.contains(token) {
            score += ADDRESS_TOKEN_BONUS;
        }
    }
    
    for phrase in locality_phrases {
        if phrase.len() >= 4 {
            let phrase_lower = phrase.to_lowercase();
            if rec.office_name_norm_lc.contains(&phrase_lower) {
                score += PHRASE_BONUS * 1.2;
            }
            if rec.full_address_norm_lc.contains(&phrase_lower) {
                score += PHRASE_BONUS * (phrase.len() as f64 / 10.0).min(1.5);
            }
        }
    }
    
    for token in query_tokens {
        let token_lower = token.to_lowercase();
        if token.len() >= 3 {
            if rec.city_norm_lc.contains(&token_lower) { score += 25.0; }
            if rec.village_norm_lc.contains(&token_lower) { score += 25.0; }
        }
    }
    
    for token in query_tokens {
        let token_lower = token.to_lowercase();
        if token.len() >= 3 {
            if rec.state_lc.contains(&token_lower) { score += 15.0; }
            if rec.district_lc.contains(&token_lower) { score += 15.0; }
        }
    }

    // SPEED FIX: Use the lazy_static PINCODE_REGEX
    if let Some(mat) = PINCODE_REGEX.find(query) {
        if mat.as_str() == rec.pincode { score += 100.0; }
    }
    
    score.min(MAX_CAP)
}

fn classify_confidence(scores: &[f64]) -> String {
    if scores.is_empty() { return "none".to_string(); }
    let best = scores[0];
    if best >= 80.0 { "high".to_string() }
    else if best >= 50.0 { "medium".to_string() }
    else if best >= 20.0 { "low".to_string() }
    else { "none".to_string() }
}

fn get_candidates(
    state: &AppState,
    q_tokens: &[String],
    locality_phrases: &[String],
) -> Vec<usize> {
    const MIN_CANDIDATES: usize = 200;

    let mut scores: Vec<(i32, usize)> = Vec::with_capacity(4096);
    let mut seen: HashSet<usize> = HashSet::with_capacity(4096);

    for phrase in locality_phrases.iter().take(5) {
        let p_tokens: Vec<&str> = phrase.split_whitespace()
            .filter(|t| t.len() >= 3).collect();

        if p_tokens.is_empty() { continue; }

        let phrase_score = 40 + p_tokens.len() as i32 * 15;

        for token in &p_tokens {
            if let Some(list) = state.inverted_index.get(*token) {
                for &idx in list.iter().take(400) {
                    if seen.len() >= MAX_CANDIDATES * 3 { break; }
                    
                    if let Some(pos) = scores.iter().position(|(_, i)| *i == idx) {
                        scores[pos].0 += phrase_score / p_tokens.len() as i32;
                    } else if seen.insert(idx) {
                        scores.push((phrase_score / p_tokens.len() as i32, idx));
                    }
                }
            }
        }
    }

    let mut sorted_tokens = q_tokens.to_vec();
    sorted_tokens.sort_by_key(|t| state.inverted_index.get(t).map_or(usize::MAX, |v| v.len()));

    for token in sorted_tokens.iter().take(8) {
        if let Some(list) = state.inverted_index.get(token) {
            for &idx in list.iter().take(400) {
                if seen.len() >= MAX_CANDIDATES * 3 { break; }
                if seen.insert(idx) {
                    scores.push((25, idx));
                }
            }
        }
    }

    if scores.len() < MIN_CANDIDATES {
        for token in q_tokens.iter() {
            if let Some(list) = state.inverted_index.get(token) {
                for &idx in list.iter().take(500) {
                    if seen.insert(idx) {
                        scores.push((10, idx));
                    }
                    if seen.len() >= MIN_CANDIDATES * 2 { break; }
                }
            }
        }
    }

    scores.sort_by_key(|(sc, _)| std::cmp::Reverse(*sc));
    let mut candidates: Vec<usize> = scores.into_iter().take(MAX_CANDIDATES).map(|(_, idx)| idx).collect();

    if candidates.len() < MIN_CANDIDATES && !q_tokens.is_empty() {
        if let Some(first) = q_tokens.first() {
            if let Some(list) = state.inverted_index.get(first) {
                for &idx in list.iter().skip(candidates.len()) {
                    if !seen.contains(&idx) {
                        candidates.push(idx);
                    }
                    if candidates.len() >= MIN_CANDIDATES { break; }
                }
            }
        }
    }

    candidates
}

// -------- TAURI COMMANDS --------

#[tauri::command]
async fn resolve_pincode(
    address: String, 
    limit: Option<usize>,
    state: State<'_, Arc<AppState>> 
) -> Result<PincodeResolutionOut, String> {
    let t0 = Instant::now();
    let limit = limit.unwrap_or(TOP_K_DEFAULT);
    let norm_query = normalize_text(&address);

    if norm_query.is_empty() {
        return Ok(PincodeResolutionOut {
            best_pincode: None, confidence: "none".into(), candidates: vec![], duration_ms: 0
        });
    }

    let locality_phrases = extract_locality_phrases(&norm_query);
    let q_tokens: Vec<String> = norm_query.split_whitespace()
        .filter(|t| t.len() >= MIN_LOCALITY_TOKEN_LEN).map(String::from).collect();

    let mut hasher = DefaultHasher::new();
    norm_query.hash(&mut hasher);
    let cache_key = hasher.finish().to_string();

    {
        let read_guard = state.cache.read().await;
        if let Some(cached) = read_guard.peek(&cache_key) {
            let candidates = cached.iter().filter_map(|(s, idx)| {
                state.index.get(*idx).map(|rec| PincodeCandidateOut {
                    pincode: rec.pincode.clone(),
                    office_name: rec.office_name.clone(),
                    city: rec.city.clone(),
                    district: rec.district.clone(),
                    state: rec.state.clone(),
                    score: *s,
                })
            }).take(limit).collect();
            
            return Ok(PincodeResolutionOut { 
                best_pincode: cached.first().and_then(|(_, i)| state.index.get(*i)).map(|r| r.pincode.clone()),
                confidence: "cache".into(), 
                candidates,
                duration_ms: t0.elapsed().as_millis() as u64
            });
        }
    }

    let candidate_indices = get_candidates(&state, &q_tokens, &locality_phrases);
    
    let mut scored: Vec<(f64, usize)> = if candidate_indices.len() <= 300 {
        candidate_indices.into_iter().map(|idx| {
            (compute_score(&state.index[idx], &address, &q_tokens, &locality_phrases), idx)
        }).collect()
    } else {
        candidate_indices.into_par_iter().map(|idx| {
            (compute_score(&state.index[idx], &address, &q_tokens, &locality_phrases), idx)
        }).collect()
    };

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    
    {
        let mut write_guard = state.cache.write().await;
        write_guard.put(cache_key, scored.clone());
    }

    let candidates: Vec<PincodeCandidateOut> = scored.into_iter().take(limit).filter_map(|(score, idx)| {
        state.index.get(idx).map(|rec| PincodeCandidateOut {
            pincode: rec.pincode.clone(),
            office_name: rec.office_name.clone(),
            city: rec.city.clone(),
            district: rec.district.clone(),
            state: rec.state.clone(),
            score,
        })
    }).collect();

    let best_pincode = candidates.first().map(|c| c.pincode.clone());
    let confidence = if let Some(first) = candidates.first() {
        if first.score > 80.0 { "high" } else if first.score > 50.0 { "medium" } else { "low" }
    } else { "none" }.to_string();

    Ok(PincodeResolutionOut { 
        best_pincode, confidence, candidates,
        duration_ms: t0.elapsed().as_millis() as u64
    })
}

#[tauri::command]
async fn bulk_pincode_process(
    input_path: String,
    output_path: String,
    column_name: String,
    window: tauri::Window,
    state: State<'_, Arc<AppState>>
) -> Result<String, String> {
    let t_start = Instant::now();
    let job_id = Uuid::new_v4().to_string();
    
    // Save initial job to DB
    {
        let db = state.db_conn.lock().map_err(|_| "DB lock error")?;
        db.execute(
            "INSERT INTO bulk_jobs (id, filename, input_path, output_path, timestamp, status, total_rows, duration_str) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, '')",
            params![
                job_id,
                PathBuf::from(&input_path).file_name().unwrap_or_default().to_string_lossy(),
                input_path,
                output_path,
                Utc::now().to_rfc3339(),
                "processing"
            ],
        ).map_err(|e| e.to_string())?;
    }

    let mut workbook: Xlsx<_> = open_workbook(&input_path)
        .map_err(|e| format!("Could not open input file: {}", e))?;
    
    let sheet_name = workbook.sheet_names().first()
        .ok_or_else(|| "No sheets found in Excel file".to_string())?.clone();
    
    let range = workbook.worksheet_range(&sheet_name)
        .map_err(|e| format!("Error reading sheet: {}", e))?;

    let headers: Vec<String> = range.rows().next()
        .ok_or_else(|| "File is empty".to_string())?
        .iter().map(|c| c.to_string()).collect();

    let addr_idx = headers.iter()
        .position(|h| h.to_lowercase() == column_name.to_lowercase())
        .ok_or_else(|| format!("Column '{}' not found in headers", column_name))?;

    // Pre-collect rows to handle parallel iteration correctly
    let all_rows: Vec<Vec<String>> = range.rows().skip(1)
        .map(|r| r.iter().map(|c| c.to_string()).collect())
        .collect();

    let total_rows = all_rows.len();
    if total_rows == 0 {
        return Err("No data rows found".to_string());
    }

    let processed_count = Arc::new(AtomicUsize::new(0));
    // SPEED FIX: No Cache inside parallel loop!
    
    // -- PROGRESS MONITORING THREAD --
    let processed_clone = processed_count.clone();
    let window_clone = window.clone();
    
    std::thread::spawn(move || {
        let mut last_processed = 0;
        let start = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(150));
            let current = processed_clone.load(Ordering::Relaxed);
            
            if current >= total_rows {
                let _ = window_clone.emit("bulk-progress", ProgressEvent {
                    percent: 100.0,
                    processed: total_rows,
                    total: total_rows,
                    eta_seconds: 0
                });
                break;
            }

            if current > last_processed {
                let elapsed = start.elapsed().as_secs_f64();
                let rate = current as f64 / elapsed; // rows per second
                let remaining = total_rows - current;
                let eta = if rate > 0.0 { (remaining as f64 / rate) as u64 } else { 0 };

                let _ = window_clone.emit("bulk-progress", ProgressEvent {
                    percent: (current as f64 / total_rows as f64) * 100.0,
                    processed: current,
                    total: total_rows,
                    eta_seconds: eta
                });
                last_processed = current;
            }
        }
    });

    // -- PARALLEL PROCESSING --
    // We batch to reduce overhead
    const BATCH_SIZE: usize = 100;
    
    let resolved_data: Vec<String> = all_rows.par_chunks(BATCH_SIZE)
        .flat_map(|batch| {
            let batch_res: Vec<String> = batch.iter().map(|row_data| {
                let address = row_data.get(addr_idx).cloned().unwrap_or_default();
                let result = if address.trim().is_empty() {
                    "N/A".to_string()
                } else {
                    // SPEED FIX: No Mutex/RwLock cache here. Local vars only.
                    let norm_query = normalize_text(&address);
                    let q_tokens: Vec<String> = norm_query.split_whitespace()
                        .filter(|t| t.len() >= MIN_LOCALITY_TOKEN_LEN).map(String::from).collect();
                    let locality_phrases = extract_locality_phrases(&norm_query);

                    let candidate_indices = get_candidates(&state, &q_tokens, &locality_phrases);
                    let mut scored: Vec<(f64, usize)> = candidate_indices.iter().map(|&idx| {
                        (compute_score(&state.index[idx], &address, &q_tokens, &locality_phrases), idx)
                    }).collect();
                    
                    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                    scored.first().map(|(_, idx)| state.index[*idx].pincode.clone()).unwrap_or_else(|| "Not Found".into())
                };

                processed_count.fetch_add(1, Ordering::Relaxed);
                result
            }).collect();
            batch_res
        })
        .collect();

    // Write Output
    let mut output = Workbook::new();
    let worksheet = output.add_worksheet();
    
    for (col, header) in headers.iter().enumerate() {
        worksheet.write(0, col as u16, header.as_str()).map_err(|e| e.to_string())?;
    }
    worksheet.write(0, headers.len() as u16, "Resolved_Pincode").map_err(|e| e.to_string())?;

    for (i, row) in all_rows.iter().enumerate() {
        let output_row = (i + 1) as u32;
        for (col_idx, value) in row.iter().enumerate() {
            worksheet.write(output_row, col_idx as u16, value.as_str()).map_err(|e| e.to_string())?;
        }
        worksheet.write(output_row, headers.len() as u16, resolved_data[i].as_str()).map_err(|e| e.to_string())?;
    }

    output.save(&output_path).map_err(|e: XlsxError| e.to_string())?;
    
    let duration = t_start.elapsed();
    let duration_str = format!("{:?}", duration);

    // Update DB to completed
    {
        let db = state.db_conn.lock().map_err(|_| "DB lock error")?;
        db.execute(
            "UPDATE bulk_jobs SET status = 'completed', total_rows = ?1, duration_str = ?2 WHERE id = ?3",
            params![total_rows, duration_str, job_id],
        ).map_err(|e| e.to_string())?;
    }

    Ok(format!("Processed {} rows in {:?}", total_rows, duration))
}

#[tauri::command]
async fn get_history(state: State<'_, Arc<AppState>>) -> Result<Vec<BulkJob>, String> {
    let db = state.db_conn.lock().map_err(|_| "DB lock error")?;
    let mut stmt = db.prepare(
        "SELECT id, filename, input_path, output_path, timestamp, status, total_rows, duration_str 
         FROM bulk_jobs ORDER BY timestamp DESC"
    ).map_err(|e| e.to_string())?;

    let jobs = stmt.query_map([], |row| {
        Ok(BulkJob {
            id: row.get(0)?,
            filename: row.get(1)?,
            input_path: row.get(2)?,
            output_path: row.get(3)?,
            timestamp: row.get(4)?,
            status: row.get(5)?,
            total_rows: row.get(6)?,
            duration_str: row.get(7)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for job in jobs {
        result.push(job.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

#[tauri::command]
async fn delete_history_item(id: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let db = state.db_conn.lock().map_err(|_| "DB lock error")?;
    
    // Get file path first
    let output_path: String = db.query_row(
        "SELECT output_path FROM bulk_jobs WHERE id = ?1",
        params![id],
        |row| row.get(0)
    ).map_err(|e| e.to_string())?;

    // Try delete file (ignore error if missing)
    let _ = std::fs::remove_file(output_path);

    // Delete DB entry
    db.execute("DELETE FROM bulk_jobs WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
        
    Ok(())
}

#[tauri::command]
async fn open_output_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd").args(&["/C", "start", "", &path]).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&path).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(&path).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}


fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_handle = app.handle().clone();
            
            // Get resource path for data loading
            let resource_path = app.path().resource_dir().unwrap_or(PathBuf::from("."));

            // Async data loading to prevent UI freeze
            tauri::async_runtime::spawn(async move {
                println!("Starting background data load...");
                match load_data(&resource_path, &app_handle) {
                    Ok(state) => {
                        app_handle.manage(Arc::new(state));
                        app_handle.emit("data-loaded", true).unwrap();
                        println!("Data load complete!");
                    }
                    Err(e) => {
                        eprintln!("Failed to load data: {}", e);
                        app_handle.emit("data-loaded", false).unwrap();
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            resolve_pincode, bulk_pincode_process, get_history, delete_history_item, open_output_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

