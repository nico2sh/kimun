/* ============================================================
   Kimün — Ask (RAG) workspace logic
   ============================================================ */
(function(){
const V = window.KIMUN_VAULT;
const TOP_K = 5;
const MODEL = 'claude-haiku';

// ---- state -------------------------------------------------
const state = {
  turns: [],           // {id,q,answer,sources,status,editing,saved,cites}
  active: -1,          // selected turn index (drives sources panel)
  focus: 'composer',   // composer | thread | sources
  src: {mode:'list', idx:0}, // sources panel: list|reader + cursor
  leader: false
};
let uid = 1;

const $ = s=>document.querySelector(s);
const thread = $('#thread');
const sourcesBody = $('#sourcesBody');
const ta = $('#composerInput');

// ---- inline markdown-ish formatter -------------------------
function esc(s){return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
function inline(s){
  s = esc(s);
  s = s.replace(/\[\[([^\]]+)\]\]/g,'<span class="wl">[[$1]]</span>');
  s = s.replace(/\[(\d+)\]/g,(m,n)=>`<sup class="cite" data-n="${n}">${n}</sup>`);
  s = s.replace(/\*\*(.+?)\*\*/g,'<b>$1</b>');
  s = s.replace(/(^|[\s(])#([a-z0-9/_-]+)/gi,'$1<span class="tag">#$2</span>');
  return s;
}
function fmtAnswer(text){
  const blocks = text.trim().split(/\n{2,}/);
  return blocks.map(b=>{
    const lines = b.split('\n');
    if(lines.every(l=>/^\s*[-*]\s+/.test(l)))
      return '<ul>'+lines.map(l=>'<li>'+inline(l.replace(/^\s*[-*]\s+/,''))+'</li>').join('')+'</ul>';
    return '<p>'+inline(b.replace(/\n/g,' '))+'</p>';
  }).join('');
}
function plain(text){return text.replace(/\[(\d+)\]/g,'').replace(/\*\*(.+?)\*\*/g,'$1').replace(/[ ]{2,}/g,' ').trim();}

// ---- retrieval + answer ------------------------------------
function retrieve(q){ return V.retrieve(q, TOP_K); }

function extractive(q, sources){
  // fallback answer: stitch the top chunks, cite them
  if(!sources.length) return "I couldn't find anything in the vault that matches that. Try rephrasing, or widen the query.";
  const lead = sources.slice(0,2).map((s,i)=>{
    const sent = s.chunk.split(/(?<=[.!?])\s+/)[0];
    return sent.replace(/\s*$/,'') + ` [${i+1}]`;
  });
  let out = lead.join('\n\n');
  if(sources.length>2) out += `\n\nRelated context also came back from **${sources[2].title}** [3].`;
  return out;
}

async function generate(q, sources, history){
  const ctx = sources.map((s,i)=>`[${i+1}] ${s.path} — "${s.h.replace(/^#+\s*/,'')}"\n${s.chunk}`).join('\n\n');
  const sys = `You are the assistant inside Kimün, a terminal note-taking app. Answer the user's question using ONLY the numbered context notes retrieved from their vault. Cite the notes you use inline with [n] matching the numbers. Keep it tight: 2–4 short paragraphs or a short bullet list, plain markdown. Preserve [[wikilinks]] and #tags from the notes. If the context does not answer the question, say so plainly.`;
  const msgs = [];
  history.forEach(t=>{ msgs.push({role:'user',content:t.q}); msgs.push({role:'assistant',content:plain(t.answer)}); });
  msgs.push({role:'user', content:`Context notes:\n\n${ctx}\n\n---\nQuestion: ${q}`});
  try{
    const out = await window.claude.complete({system:sys, max_tokens:700, messages:msgs});
    if(out && out.trim()) return out.trim();
  }catch(e){ /* fall through */ }
  return extractive(q, sources);
}

// ---- ask flow ----------------------------------------------
async function ask(q){
  q = q.trim(); if(!q) return;
  const sources = retrieve(q);
  const turn = {id:uid++, q, answer:'', sources, status:'thinking', editing:false, saved:null, cites:new Set()};
  state.turns.push(turn);
  state.active = state.turns.length-1;
  state.src = {mode:'list', idx:0};
  ta.value=''; sizeTa();
  render(); scrollBottom();

  const history = state.turns.slice(0,-1).filter(t=>t.status==='done');
  turn.status='streaming';
  const full = await generate(q, sources, history);
  await stream(turn, full);
  turn.status='done';
  renderTurn(turn); renderSources();
}

function stream(turn, full){
  return new Promise(res=>{
    const words = full.split(/(\s+)/);
    let i=0;
    (function step(){
      i += 2;
      turn.answer = words.slice(0,i).join('');
      renderTurn(turn, true);
      if(state.active===state.turns.indexOf(turn)) autoscroll();
      if(i<words.length) setTimeout(step, 16);
      else { turn.answer = full; renderTurn(turn); res(); }
    })();
  });
}

// ---- rendering ---------------------------------------------
function render(){
  if(!state.turns.length){ thread.innerHTML = emptyState(); }
  else thread.innerHTML = state.turns.map(turnHtml).join('');
  renderSources(); syncFocus();
}
function renderTurn(turn, streaming){
  const el = thread.querySelector(`.turn[data-id="${turn.id}"]`);
  if(el) el.outerHTML = turnHtml(turn, streaming);
  else { render(); }
}

function emptyState(){
  const ex = [
    'What did we decide about auth token rotation, and who owns it?',
    'Summarize the search caching rollout status.',
    'What are the open risks with Redis client-side caching?'
  ];
  return `<div class="empty"><div class="big">Ask your vault.</div>
    <div>Natural-language questions run over the vector index; the top ${TOP_K} note chunks are retrieved and sent to ${MODEL} as context. Answers cite their sources.</div>
    <div class="ex">${ex.map(q=>`<div class="q" data-ask="${q.replace(/"/g,'&quot;')}"><span class="pfx">\u276f</span>${q}</div>`).join('')}</div>
  </div>`;
}

function turnHtml(turn, streaming){
  const sel = state.turns.indexOf(turn)===state.active ? ' sel':'';
  let ansInner;
  if(turn.status==='thinking'){
    ansInner = `<div class="lbl"><span class="spin">\u25cf</span> retrieving \u00b7 ${turn.sources.length} chunks \u2192 ${MODEL}\u2026</div>`;
  } else if(turn.editing){
    ansInner = `<div class="lbl">editing as note</div>
      <div class="noteedit" contenteditable="true" spellcheck="false">${esc(plain(turn.answer))}</div>
      <div class="editbar"><span><b>Ctrl-S</b> save to vault</span><span><b>Esc</b> cancel</span></div>`;
  } else {
    const cur = streaming ? '<span class="cur-blink">&nbsp;</span>' : '';
    ansInner = `<div class="lbl">answer</div><div class="body">${fmtAnswer(turn.answer)}${cur}</div>`;
    if(turn.status==='done') ansInner += foot(turn);
  }
  return `<div class="turn${sel}" data-id="${turn.id}">
    <div class="qline"><span class="pfx">\u276f</span><span class="qt">${esc(turn.q)}</span></div>
    <div class="ans">${ansInner}</div>
  </div>`;
}

function foot(turn){
  const pills = turn.sources.map((s,i)=>`<span class="pill" data-src="${i}"><span class="n">${i+1}</span>${s.path}<span class="sc">${(s.score*100|0)}%</span></span>`).join('');
  const saved = turn.saved ? `<span class="saved">saved \u2192 ${turn.saved}</span>` : '';
  return `<div class="ansfoot">${pills}<span class="gap"></span>${saved}
    <span class="act" data-act="copy"><b>y</b> copy</span>
    <span class="act" data-act="edit"><b>e</b> edit as note</span>
    <span class="act" data-act="regen"><b>r</b> regenerate</span></div>`;
}

// ---- sources panel -----------------------------------------
function renderSources(){
  const turn = state.turns[state.active];
  if(!turn){ sourcesBody.innerHTML = `<div class="srchead">no query selected</div><div class="srclist"><div class="src"><div class="path">ask a question to retrieve context</div></div></div>`; return; }
  if(state.src.mode==='reader') sourcesBody.innerHTML = readerHtml(turn);
  else sourcesBody.innerHTML = listHtml(turn);
}
function listHtml(turn){
  const head = `<div class="srchead">context <span class="cnt">top ${turn.sources.length}</span><span class="sp"></span>similarity</div>`;
  const rows = turn.sources.map((s,i)=>{
    const cur = (state.focus==='sources' && state.src.idx===i)?' cursor':'';
    const on = turn.cites.has(String(i+1))?'':''; // cites highlighted via .on toggled dynamically
    const snip = highlightSnip(s.chunk, turn.q);
    return `<div class="src${cur}" data-idx="${i}">
      <div class="top"><span class="rank">${i+1}</span><span class="ttl">${s.title}</span><span class="pct">${(s.score*100).toFixed(0)}%</span></div>
      <div class="path">${s.path}</div>
      <div class="scorebar"><i style="width:${(s.score*100).toFixed(0)}%"></i></div>
      <div class="snip">${snip}</div>
    </div>`;
  }).join('');
  return head + `<div class="srclist">${rows}</div>`;
}
function highlightSnip(text, q){
  const words = V.toks(q);
  let s = esc(text);
  words.forEach(w=>{ if(w.length<3)return; s = s.replace(new RegExp('\\b('+w.replace(/[.*+?^${}()|[\]\\]/g,'')+'\\w*)','ig'),'<b>$1</b>'); });
  return s;
}
function readerHtml(turn){
  const s = turn.sources[state.src.idx];
  const note = V.noteById(s.noteId);
  const body = note.chunks.map(c=>{
    const isHit = c.t===s.chunk;
    const hRendered = c.h.startsWith('# ')?`<div class="h1">${esc(c.h)}</div>`
      : c.h.startsWith('## ')?`<div class="h2">${esc(c.h)}</div>`
      : `<div class="h3">${esc(c.h)}</div>`;
    const p = `<div class="p">${inline(c.t)}</div>`;
    return isHit ? `<div class="hl">${hRendered}${p}</div>` : hRendered+p;
  }).join('');
  return `<div class="rd-head"><span class="rank">${state.src.idx+1}</span><span class="fn">${s.path}</span><span class="pct">${(s.score*100).toFixed(0)}% match</span></div>
    <div class="rd-body"><div class="ed">${body}</div></div>
    <div class="rd-foot"><span><b>h</b>/<b>Esc</b> back</span><span><b>j/k</b> other source</span><span><b>o</b> open in editor</span><span><b>y</b> yank path</span></div>`;
}

// ---- focus / selection -------------------------------------
function syncFocus(){
  $('#threadPanel').classList.toggle('focus', state.focus==='thread');
  $('#composerPanel').classList.toggle('focus', state.focus==='composer');
  $('#sourcesPanel').classList.toggle('focus', state.focus==='sources');
  $('#modeChip').textContent = state.focus==='composer'?'\u2726 ASK':state.focus==='thread'?'\u2261 THREAD':'\u25a4 SOURCES';
  if(state.focus==='composer') ta.focus(); else ta.blur();
  updateStatus();
}
function setFocus(z){ state.focus=z; if(z==='sources' && state.active<0 && state.turns.length){state.active=state.turns.length-1;} syncFocus(); if(z!=='composer'){renderSources();} }
function setActive(i){
  i = Math.max(0, Math.min(state.turns.length-1, i));
  state.active = i; state.src={mode:'list',idx:0};
  render();
  const el = thread.querySelector(`.turn[data-id="${state.turns[i].id}"]`);
  if(el) el.scrollIntoViewIfNeeded ? el.scrollIntoViewIfNeeded() : el.scrollTop;
}

// ---- actions -----------------------------------------------
function copyAnswer(turn){ if(!turn)return; navigator.clipboard?.writeText(plain(turn.answer)); toast('answer copied to clipboard'); }
function editAnswer(turn){ if(!turn||turn.status!=='done')return; turn.editing=true; renderTurn(turn);
  setTimeout(()=>{ const ne=thread.querySelector(`.turn[data-id="${turn.id}"] .noteedit`); if(ne){ne.focus(); placeCaretEnd(ne);} },0); }
function saveNote(turn){
  const ne = thread.querySelector(`.turn[data-id="${turn.id}"] .noteedit`);
  if(ne) turn.answer = ne.innerText;
  turn.editing=false;
  const slug = turn.q.toLowerCase().replace(/[^a-z0-9]+/g,'-').replace(/^-|-$/g,'').slice(0,32);
  turn.saved = `ask/${slug||'answer'}.md`;
  renderTurn(turn); toast('saved to vault \u00b7 '+turn.saved);
}
function cancelEdit(turn){ turn.editing=false; renderTurn(turn); }
async function regen(turn){
  if(!turn) return; turn.status='streaming'; turn.answer=''; turn.saved=null; renderTurn(turn);
  const history = state.turns.slice(0,state.turns.indexOf(turn)).filter(t=>t.status==='done');
  const full = await generate(turn.q, turn.sources, history);
  await stream(turn, full); turn.status='done'; renderTurn(turn);
}
function openInEditor(s){ toast('opened '+s.path+' in editor'); }

function placeCaretEnd(el){ const r=document.createRange(); r.selectNodeContents(el); r.collapse(false); const sel=getComputedStyle?window.getSelection():null; if(sel){sel.removeAllRanges();sel.addRange(r);} }

// ---- toast -------------------------------------------------
let toastT;
function toast(msg){ const t=$('#toast'); t.innerHTML=msg; t.classList.add('on'); clearTimeout(toastT); toastT=setTimeout(()=>t.classList.remove('on'),1700); }

// ---- scroll ------------------------------------------------
function scrollBottom(){ thread.scrollTop = thread.scrollHeight; }
function autoscroll(){ const nearBottom = thread.scrollHeight-thread.scrollTop-thread.clientHeight < 120; if(nearBottom) thread.scrollTop = thread.scrollHeight; }

// ---- composer sizing ---------------------------------------
function sizeTa(){ ta.style.height='auto'; ta.style.height=Math.min(ta.scrollHeight,120)+'px'; }
ta.addEventListener('input', sizeTa);

// ---- which-key + help --------------------------------------
function toggleLeader(on){ state.leader=on; $('#whichkey').style.display=on?'block':'none'; }
function toggleHelp(on){ $('#helpOv').classList.toggle('on', on!==undefined?on:!$('#helpOv').classList.contains('on')); }

// ---- keyboard ----------------------------------------------
document.addEventListener('keydown', e=>{
  const editing = state.turns[state.active] && state.turns[state.active].editing;
  const inNoteEdit = e.target.classList && e.target.classList.contains('noteedit');

  // note-edit mode captures Ctrl-S / Esc
  if(inNoteEdit){
    if((e.ctrlKey||e.metaKey)&&e.key==='s'){e.preventDefault();saveNote(state.turns[state.active]);}
    else if(e.key==='Escape'){e.preventDefault();cancelEdit(state.turns[state.active]);setFocus('thread');}
    return;
  }

  // help overlay
  if($('#helpOv').classList.contains('on')){ if(e.key==='Escape'||e.key==='?'){toggleHelp(false);e.preventDefault();} return; }

  // leader (Ctrl-K, or Space when a non-text panel is focused)
  if(state.leader){
    e.preventDefault(); toggleLeader(false);
    const k=e.key.toLowerCase();
    const t=state.turns[state.active];
    if(k==='a'){setFocus('composer');}
    else if(k==='n'){newConversation();}
    else if(k==='y'){copyAnswer(t);}
    else if(k==='e'){editAnswer(t);}
    else if(k==='r'){regen(t);}
    else if(k==='s'){if(t){setFocus('sources');openReader(state.src.idx||0);}}
    else if(k==='?'){toggleHelp(true);}
    return;
  }
  if((e.ctrlKey||e.metaKey)&&e.key.toLowerCase()==='k'){e.preventDefault();toggleLeader(true);return;}

  // ---- composer focused ----
  if(state.focus==='composer'){
    if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();ask(ta.value);}
    else if(e.key==='Escape'){e.preventDefault();if(state.turns.length){setFocus('thread');}}
    else if(e.key==='Tab'){e.preventDefault();setFocus(e.shiftKey?'sources':'thread');}
    return;
  }

  // ---- non-text panels ----
  if(e.key==='Tab'){e.preventDefault();cycle(e.shiftKey?-1:1);return;}
  if(e.key===' '){e.preventDefault();toggleLeader(true);return;}
  if(e.key==='?'){e.preventDefault();toggleHelp(true);return;}
  if(e.key==='i'||e.key==='/'){e.preventDefault();setFocus('composer');return;}

  if(state.focus==='thread'){
    const t=state.turns[state.active];
    if(e.key==='j'||e.key==='ArrowDown'){e.preventDefault();setActive(state.active+1);}
    else if(e.key==='k'||e.key==='ArrowUp'){e.preventDefault();setActive(state.active-1);}
    else if(e.key==='Enter'||e.key==='l'){e.preventDefault();setFocus('sources');}
    else if(e.key==='y'){e.preventDefault();copyAnswer(t);}
    else if(e.key==='e'){e.preventDefault();editAnswer(t);}
    else if(e.key==='r'){e.preventDefault();regen(t);}
    return;
  }

  if(state.focus==='sources'){
    const turn=state.turns[state.active]; if(!turn)return;
    if(state.src.mode==='list'){
      if(e.key==='j'||e.key==='ArrowDown'){e.preventDefault();moveSrc(1);}
      else if(e.key==='k'||e.key==='ArrowUp'){e.preventDefault();moveSrc(-1);}
      else if(e.key==='Enter'||e.key==='l'){e.preventDefault();openReader(state.src.idx);}
      else if(e.key==='h'||e.key==='Escape'){e.preventDefault();setFocus('thread');}
    } else { // reader
      if(e.key==='h'||e.key==='Escape'){e.preventDefault();state.src.mode='list';renderSources();}
      else if(e.key==='j'||e.key==='ArrowDown'){e.preventDefault();state.src.idx=Math.min(turn.sources.length-1,state.src.idx+1);renderSources();}
      else if(e.key==='k'||e.key==='ArrowUp'){e.preventDefault();state.src.idx=Math.max(0,state.src.idx-1);renderSources();}
      else if(e.key==='o'){e.preventDefault();openInEditor(turn.sources[state.src.idx]);}
      else if(e.key==='y'){e.preventDefault();navigator.clipboard?.writeText(turn.sources[state.src.idx].path);toast('yanked path');}
    }
    return;
  }
});

function cycle(dir){ const order=['composer','thread','sources']; let i=order.indexOf(state.focus); i=(i+dir+3)%3; setFocus(order[i]); }
function moveSrc(d){ const turn=state.turns[state.active]; state.src.idx=Math.max(0,Math.min(turn.sources.length-1,state.src.idx+d)); renderSources(); }
function openReader(i){ state.src.mode='reader'; state.src.idx=i; setFocus('sources'); renderSources(); }
function newConversation(){ state.turns=[]; state.active=-1; state.src={mode:'list',idx:0}; render(); setFocus('composer'); }

// ---- mouse -------------------------------------------------
thread.addEventListener('click', e=>{
  const askq = e.target.closest('[data-ask]'); if(askq){ask(askq.dataset.ask);return;}
  const cite = e.target.closest('.cite'); if(cite){ const turnEl=cite.closest('.turn'); selectTurnEl(turnEl); openReader(parseInt(cite.dataset.n)-1); return; }
  const pill = e.target.closest('.pill'); if(pill){ const turnEl=pill.closest('.turn'); selectTurnEl(turnEl); openReader(parseInt(pill.dataset.src)); return; }
  const act = e.target.closest('.act'); if(act){ const turnEl=act.closest('.turn'); selectTurnEl(turnEl); const t=state.turns[state.active]; const a=act.dataset.act; if(a==='copy')copyAnswer(t);else if(a==='edit')editAnswer(t);else if(a==='regen')regen(t); return; }
  const turnEl = e.target.closest('.turn'); if(turnEl){ selectTurnEl(turnEl); setFocus('thread'); }
});
function selectTurnEl(turnEl){ const id=+turnEl.dataset.id; const i=state.turns.findIndex(t=>t.id===id); if(i>=0&&i!==state.active){state.active=i;state.src={mode:'list',idx:0};render();} }

sourcesBody.addEventListener('click', e=>{
  const src=e.target.closest('.src[data-idx]'); if(src){ setFocus('sources'); openReader(+src.dataset.idx); }
});
$('#composerPanel').addEventListener('click', ()=>setFocus('composer'));
$('#helpOv').addEventListener('click', e=>{ if(e.target.id==='helpOv')toggleHelp(false); });
$('#railAsk')&&$('#railAsk').addEventListener('click',()=>setFocus('composer'));

function updateStatus(){
  const l1=$('#statusKeys');
  const map={
    composer:'<span class="key"><b>\u21b5</b> ask</span><span class="key"><b>\u21e7\u21b5</b> newline</span><span class="key"><b>\u21e5</b> thread</span><span class="key"><b>Ctrl-K</b> menu</span>',
    thread:'<span class="key"><b>j/k</b> turns</span><span class="key"><b>\u21b5</b> sources</span><span class="key"><b>y</b> copy</span><span class="key"><b>e</b> edit</span><span class="key"><b>r</b> regen</span><span class="key"><b>i</b> ask</span>',
    sources:'<span class="key"><b>j/k</b> move</span><span class="key"><b>\u21b5</b> open</span><span class="key"><b>h</b> back</span><span class="key"><b>o</b> editor</span><span class="key"><b>i</b> ask</span>'
  };
  l1.innerHTML = map[state.focus] + '<span class="sp"></span><span class="key"><b>Tab</b> focus</span><span class="key"><b>?</b> help</span>';
}

// ---- seed --------------------------------------------------
function pickChunk(noteId, idx){ const n=V.noteById(noteId); const c=n.chunks[idx]; return {noteId, path:n.path, title:n.title, mtime:n.mtime, h:c.h, chunk:c.t, ci:idx}; }
function seed(){
  const q='What did we decide about auth token rotation, and who owns the migration?';
  // deterministic top_k so the seeded citations line up exactly
  const sources = [
    Object.assign(pickChunk('auth-flow',1), {score:0.91}),
    Object.assign(pickChunk('search-caching',2), {score:0.74}),
    Object.assign(pickChunk('maria',1), {score:0.72}),
    Object.assign(pickChunk('auth-flow',2), {score:0.69}),
    Object.assign(pickChunk('sprint-planning',1), {score:0.61})
  ];
  const answer = `We decided to rotate **refresh tokens** on a fixed 24-hour schedule instead of on every request, and to keep access tokens short-lived at 15 minutes — small blast radius without hammering the token service. [1]

Ownership sits with **[[maria]]**: she's driving the migration end to end and tracking the refresh-flow follow-up, targeting the new rotation live behind a flag next sprint. [1][3]

Good news for the caching work — token rotation doesn't touch the cache keys, so the two rollouts are independent and can ramp separately behind their own flags. [2]`;
  state.turns.push({id:uid++, q, answer, sources, status:'done', editing:false, saved:null, cites:new Set()});
  state.active=0;
}

seed();
render();
setFocus('composer');
updateStatus();
})();
