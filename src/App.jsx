import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { 
  Search, FileUp, History, Trash2, MapPin, 
  CheckCircle2, AlertCircle, Loader2, FileText, 
  Database, Download, Globe, Clock, BarChart3, X
} from "lucide-react";

export default function App() {
  const [activeTab, setActiveTab] = useState("search");
  const [address, setAddress] = useState("");
  const [results, setResults] = useState(null);
  const [history, setHistory] = useState([]);
  const [lastOutputPath, setLastOutputPath] = useState(null); 
  
  // Bulk Processing State
  const [isProcessing, setIsProcessing] = useState(false);
  const [columnName, setColumnName] = useState("Address");
  const [selectedFile, setSelectedFile] = useState(null);
  const [bulkStats, setBulkStats] = useState({ percent: 0, processed: 0, total: 0, eta: 0 });
  
  // App State
  const [notification, setNotification] = useState(null);
  const [isAppReady, setIsAppReady] = useState(false);
  const [loadProgress, setLoadProgress] = useState(0);

  useEffect(() => {
    // Listeners
    const unlistenLoaded = listen("data-loaded", (e) => {
      if(e.payload) { setIsAppReady(true); showNotify('success', "Database Ready"); }
      else { showNotify('error', "Database Load Failed"); }
    });

    const unlistenLoadProg = listen("loading-progress", (e) => {
      // Approx 155k records for full bar
      setLoadProgress(Math.min((e.payload / 155000) * 100, 100));
    });

    const unlistenBulk = listen("bulk-progress", (e) => {
      setBulkStats({
        percent: e.payload.percent,
        processed: e.payload.processed,
        total: e.payload.total,
        eta: e.payload.eta_seconds
      });
    });

    return () => {
      unlistenLoaded.then(f => f());
      unlistenLoadProg.then(f => f());
      unlistenBulk.then(f => f());
    };
  }, []);

  useEffect(() => {
    if (activeTab === "history" && isAppReady) fetchHistory();
  }, [activeTab, isAppReady]);

  const fetchHistory = async () => {
    try { setHistory(await invoke("get_history")); } catch(e) { console.error(e); }
  };

  const showNotify = useCallback((type, message) => {
    setNotification({ type, message });
    setTimeout(() => setNotification(null), 4000);
  }, []);

  const handlePickFile = async () => {
    try {
      const selected = await open({ filters: [{ name: 'Excel', extensions: ['xlsx', 'xls'] }] });
      if (selected) setSelectedFile(selected);
    } catch { showNotify('error', "Selection failed"); }
  };

  const handleSearch = async () => {
    if (!address.trim()) return;
    try {
      setResults(await invoke("resolve_pincode", { address, limit: 5 }));
    } catch(e) { showNotify('error', e); }
  };

  const handleProcessFile = async () => {
    if (!selectedFile) return;
    setIsProcessing(true);
    setBulkStats({ percent: 0, processed: 0, total: 0, eta: 0 });
    
    try {
      const outputPath = selectedFile.replace(/\.[^/.]+$/, "") + "_resolved.xlsx";
      await invoke("bulk_pincode_process", { inputPath: selectedFile, outputPath, columnName });
      showNotify('success', "Processing Complete");
      // Update state to show success UI instead of resetting
      setLastOutputPath(outputPath); 
      setIsProcessing(false);
    //   setTimeout(() => { setIsProcessing(false); setSelectedFile(null); }, 1000);
    } catch(e) {
      showNotify('error', e);
      setIsProcessing(false);
    }
  };

  const handleDelete = async (id) => {
    await invoke("delete_history_item", { id });
    setHistory(prev => prev.filter(h => h.id !== id));
  };

  const handleOpenFile = async (path) => {
    try {
      console.log("Attempting to open:", path); // Debug log
      await invoke("open_output_file", { path });
    } catch(err) {
      console.error(err);
      showNotify('error', "Could not open file: " + err);
    }
  }
  
  // Loading Screen
  if (!isAppReady) {
    return (
      <div className="h-screen w-full bg-[#0F172A] flex flex-col items-center justify-center">
        <Loader2 className="animate-spin text-blue-500 mb-4" size={48} />
        <h2 className="text-white text-xl font-bold">Initializing Database...</h2>
        <div className="w-64 h-1.5 bg-slate-800 rounded-full mt-6 overflow-hidden">
          <div className="h-full bg-blue-500 transition-all duration-300" style={{width: `${loadProgress}%`}}></div>
        </div>
        <p className="text-slate-500 text-xs mt-2">{Math.round(loadProgress)}%</p>
      </div>
    );
  }

  return (
    <div className="flex h-screen bg-slate-50 font-sans text-slate-900">
      {/* Sidebar */}
      <aside className="w-64 bg-[#0F172A] flex flex-col shadow-xl text-white">
        <div className="p-6">
          <div className="flex items-center gap-3 mb-8">
            <div className="bg-blue-600 p-2 rounded-lg"><Globe size={20}/></div>
            <span className="font-bold text-lg">PinMaster</span>
          </div>
          <nav className="space-y-2">
            <NavBtn active={activeTab==="search"} onClick={()=>setActiveTab("search")} icon={<Search/>} label="Lookup"/>
            <NavBtn active={activeTab==="bulk"} onClick={()=>setActiveTab("bulk")} icon={<FileUp/>} label="Bulk Excel"/>
            <NavBtn active={activeTab==="history"} onClick={()=>setActiveTab("history")} icon={<History/>} label="History"/>
          </nav>
        </div>
      </aside>

      {/* Main */}
      <main className="flex-1 overflow-y-auto p-10 relative">
        {notification && (
          <div className={`absolute top-5 right-5 px-6 py-3 rounded-xl shadow-xl flex items-center gap-3 animate-in fade-in slide-in-from-top-5 ${notification.type==='success'?'bg-white text-green-600':'bg-white text-red-600'}`}>
            {notification.type==='success' ? <CheckCircle2/> : <AlertCircle/>}
            <span className="font-bold">{notification.message}</span>
          </div>
        )}

        {activeTab === "search" && (
          <div className="max-w-3xl mx-auto mt-10">
            <h2 className="text-3xl font-black mb-6">Address Lookup</h2>
            <div className="flex gap-4 mb-10">
              <input value={address} onChange={e=>setAddress(e.target.value)} onKeyDown={e=>e.key==='Enter'&&handleSearch()} 
                className="flex-1 p-4 rounded-xl border border-slate-200 shadow-sm outline-none focus:ring-2 focus:ring-blue-500" placeholder="Enter address..."/>
              <button onClick={handleSearch} className="bg-blue-600 text-white px-8 rounded-xl font-bold hover:bg-blue-700 transition">Search</button>
            </div>
            {results && (
              <div className="space-y-4 animate-in slide-in-from-bottom-5">
                <div className="bg-white p-6 rounded-2xl border border-blue-100 shadow-lg shadow-blue-500/5">
                    <div className="flex justify-between items-start">
                        <div>
                        <div className="text-xs font-bold text-blue-500 uppercase tracking-widest mb-1">Best Match</div>
                        <div className="text-4xl font-black">{results.best_pincode || "Not Found"}</div>
                        <div className="text-slate-400 text-sm font-bold mt-2">Confidence: {results.confidence}</div>
                        </div>
                        <div className="text-right bg-slate-50 px-3 py-2 rounded-lg">
                        <div className="text-[10px] font-bold text-slate-400 uppercase tracking-widest">Time</div>
                        <div className="font-mono font-bold text-slate-600">{results.duration_ms}ms</div>
                        </div>
                    </div>
                    </div>
                {results.candidates.map((c,i) => (
                  <div key={i} className="bg-white p-4 rounded-xl border border-slate-100 flex justify-between items-center">
                    <div>
                      <div className="font-bold text-lg">{c.pincode}</div>
                      <div className="text-slate-500 text-sm">{c.office_name}</div>
                    </div>
                    <div className="bg-slate-100 px-3 py-1 rounded-lg text-xs font-bold">{c.score.toFixed(0)}</div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {activeTab === "bulk" && (
          <div className="max-w-xl mx-auto mt-20 text-center">
            <h2 className="text-3xl font-black mb-2">Bulk Processing</h2>
            <p className="text-slate-500 mb-10">Process thousands of rows in seconds.</p>
            
            <div className={`border-2 border-dashed rounded-3xl p-10 transition-all cursor-pointer ${selectedFile ? 'border-blue-500 bg-blue-50/30' : 'border-slate-300 hover:border-blue-400'}`}
                 onClick={!isProcessing ? handlePickFile : undefined}>
              <FileText className={`mx-auto mb-4 ${selectedFile?'text-blue-500':'text-slate-300'}`} size={48}/>
              <div className="font-bold text-lg">{selectedFile ? selectedFile.split(/[\\/]/).pop() : "Select Excel File"}</div>
            </div>

            {isProcessing ? (
              <div className="mt-8 text-left bg-white p-6 rounded-2xl shadow-sm border border-slate-100">
                <div className="flex justify-between mb-2">
                  <span className="font-bold text-slate-700">Processing...</span>
                  <span className="font-mono text-blue-600">{bulkStats.eta}s remaining</span>
                </div>
                <div className="h-3 bg-slate-100 rounded-full overflow-hidden mb-2">
                  <div className="h-full bg-blue-600 transition-all duration-300" style={{width: `${bulkStats.percent}%`}}></div>
                </div>
                <div className="text-xs text-slate-400 font-bold text-right">{bulkStats.processed} / {bulkStats.total} Rows</div>
              </div>
            ) : lastOutputPath ? (
              <div className="mt-8 bg-white p-8 rounded-[2rem] shadow-xl shadow-green-900/5 border border-green-100 text-center animate-in zoom-in-95 duration-300">
                <div className="w-20 h-20 bg-green-50 text-green-600 rounded-full flex items-center justify-center mx-auto mb-4 shadow-inner">
                  <CheckCircle2 size={40} />
                </div>
                <h3 className="text-2xl font-black text-slate-800 mb-2">Processing Complete!</h3>
                <p className="text-slate-500 font-medium mb-8">Your data has been successfully resolved.</p>
                
                <div className="flex flex-col gap-3">
                  <button 
                    onClick={() => handleOpenFile(lastOutputPath)}
                    className="w-full py-4 bg-blue-600 hover:bg-blue-700 text-white rounded-xl font-bold flex items-center justify-center gap-2 shadow-lg shadow-blue-600/20 transition-all active:scale-95"
                  >
                    <Download size={20} /> Open Resolved File
                  </button>
                  <button 
                    onClick={() => { setLastOutputPath(null); setSelectedFile(null); }}
                    className="w-full py-4 bg-slate-100 hover:bg-slate-200 text-slate-600 rounded-xl font-bold transition-all active:scale-95"
                  >
                    Process Another File
                  </button>
                </div>
              </div>
            ) : (
              <div className="mt-6 flex gap-4">
                <input value={columnName} onChange={e=>setColumnName(e.target.value)} className="flex-1 p-4 rounded-xl border border-slate-200 font-medium" placeholder="Column Name (e.g. Address)"/>
                <button onClick={handleProcessFile} disabled={!selectedFile} className="bg-slate-900 text-white px-8 rounded-xl font-bold disabled:opacity-50 shadow-lg hover:shadow-xl transition-all">Start</button>
              </div>
            )}
          </div>
        )}

        {activeTab === "history" && (
          <div className="max-w-4xl mx-auto mt-10">
            <h2 className="text-3xl font-black mb-8">History</h2>
            <div className="space-y-4">
              {history.map(h => (
                <div key={h.id} className="bg-white p-5 rounded-2xl border border-slate-100 shadow-sm flex items-center justify-between">
                  <div className="flex items-center gap-4">
                    <div className={`p-3 rounded-xl ${h.status==='completed'?'bg-green-50 text-green-600':'bg-yellow-50 text-yellow-600'}`}>
                      {h.status==='completed'?<CheckCircle2/>:<Loader2/>}
                    </div>
                    <div>
                      <div className="font-bold text-lg">{h.filename}</div>
                      <div className="text-xs font-bold text-slate-400 uppercase tracking-widest flex gap-4">
                         <span>{new Date(h.timestamp).toLocaleDateString()}</span>
                         <span>{h.total_rows} Rows</span>
                         <span>{h.duration_str}</span>
                      </div>
                    </div>
                  </div>
                  <div className="flex gap-2">
                    {h.status==='completed' && (
                      <button onClick={()=>invoke("open_output_file",{path:h.output_path})} className="p-2 hover:bg-slate-50 rounded-lg text-blue-600"><Download/></button>
                    )}
                    <button onClick={()=>handleDelete(h.id)} className="p-2 hover:bg-red-50 rounded-lg text-red-500"><Trash2/></button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

function NavBtn({active, onClick, icon, label}) {
  return (
    <button onClick={onClick} className={`w-full flex items-center gap-3 p-3 rounded-xl font-bold transition ${active?'bg-blue-600 shadow-lg shadow-blue-900/50':'hover:bg-white/10 opacity-70 hover:opacity-100'}`}>
      {icon} <span>{label}</span>
    </button>
  );
}