/* ============================================================
   Kimün — sample vault + mock vector retrieval
   A tiny, coherent work-vault so retrieval + answers are legible.
   Retrieval here is a stand-in for the real vector DB: it tokenizes,
   scores chunk/query term overlap, and returns a plausible top_k.
   ============================================================ */
window.KIMUN_VAULT = (function(){

  // each note is split into titled chunks; chunks are what get embedded/retrieved
  const notes = [
    {
      id:'auth-flow', path:'meetings/auth-flow.md', title:'Auth Flow Meeting', mtime:'2026-04-08',
      chunks:[
        {h:'# Auth Flow Meeting', t:'2026-04-08 · attendees: [[maria]], [[david]], [[carlos]]. Topic: hardening the session layer before the flag rollout.'},
        {h:'## Decisions', t:'We decided to rotate refresh tokens on a fixed 24h schedule instead of on every request. Access tokens stay short-lived at 15 minutes. This keeps the blast radius small without hammering the token service.'},
        {h:'## Ownership', t:'[[maria]] owns the migration end to end and will track the refresh-flow follow-up. #followup Target is to have the new rotation live behind a flag by next sprint.'},
        {h:'## Open questions', t:'Do we invalidate the whole session on password change, or just the refresh token family? Parked for the next 1:1.'}
      ]
    },
    {
      id:'maria', path:'people/maria.md', title:'Maria', mtime:'2026-04-05',
      chunks:[
        {h:'# Maria', t:'Staff engineer, identity & platform. Owns the auth token rotation migration and the session service. Best reached async; syncs on Tuesdays.'},
        {h:'## Current', t:'Driving refresh-token rotation ([[auth-flow]]). Also reviewing the observability proposal for the token service. #auth #owner'}
      ]
    },
    {
      id:'search-caching', path:'notes/search-caching.md', title:'Search Caching Design', mtime:'2026-04-11',
      chunks:[
        {h:'# Search Caching Design', t:'Cache scored search results in Redis keyed by the normalized query. Goal: cut p95 latency on repeat queries without stale answers.'},
        {h:'## Shadow mode', t:'Deployed in shadow mode at 10am. Over 15,000 requests the cache-agreement rate was 99.7% with no errors and no perf regression. Proceeding to 10% live traffic tomorrow.'},
        {h:'## Invalidation', t:'Invalidate on note write via the index hook. TTL is a safety net at 6h. Token-rotation changes do not touch cache keys, so the auth work is independent. #rollout'}
      ]
    },
    {
      id:'sprint-planning', path:'meetings/sprint-planning.md', title:'Sprint Planning', mtime:'2026-04-10',
      chunks:[
        {h:'# Sprint Planning', t:'Sprint 24 goals: ship feature-flag rollout, land refresh-token rotation behind a flag, and start the observability proposal spike.'},
        {h:'## Feature flags', t:'Feature flags gate both the caching rollout and the token rotation so we can ramp independently and roll back fast. [[search-caching]]'}
      ]
    },
    {
      id:'observability', path:'proposals/observability.md', title:'Observability Proposal', mtime:'2026-04-02',
      chunks:[
        {h:'# Observability Proposal', t:'Add structured tracing across the token service and search path. Emit spans for retrieval, cache hit/miss, and token refresh.'},
        {h:'## Metrics', t:'Track cache-hit ratio, refresh-token rotation lag, and p95 query latency as the three headline SLOs. Dashboards live in Grafana. #investigate'}
      ]
    },
    {
      id:'redis-client-cache', path:'notes/redis-client-cache.md', title:'Redis 7.2 Client-side Caching', mtime:'2026-04-11',
      chunks:[
        {h:'# Redis 7.2 client-side caching', t:'[[carlos]] flagged Redis 7.2 client-side caching (RESP3 tracking) as a way to shave a network hop on hot search keys. #investigate'},
        {h:'## Risk', t:'Client-side invalidation is push-based; needs care so a stale local cache never serves a result the server invalidated. Spike before adopting.'}
      ]
    },
    {
      id:'tokenizer', path:'notes/tokenizer.md', title:'Tokenizer PR', mtime:'2026-04-11',
      chunks:[
        {h:'# Tokenizer PR', t:'Reviewed [[david]]\u2019s tokenizer PR — approved. Splits on unicode boundaries and folds case before indexing.'},
        {h:'## Follow-up', t:'Move the stop-word list into a config file so vaults can tune it per language. Small follow-up, not blocking.'}
      ]
    },
    {
      id:'daily-0411', path:'daily/2026-04-11.md', title:'2026-04-11', mtime:'2026-04-11',
      chunks:[
        {h:'## Standup', t:'Yesterday: cache invalidation tests passing, PR ready. Today: start feature-flag rollout. Blockers: none.'},
        {h:'## Rollout', t:'Deployed search caching in shadow mode at 10am — see [[search-caching]]. Cache agreement 99.7%. Proceeding to 10% traffic tomorrow.'}
      ]
    }
  ];

  // flatten to a chunk index
  const index = [];
  notes.forEach(n=>n.chunks.forEach((c,ci)=>{
    index.push({noteId:n.id, path:n.path, title:n.title, mtime:n.mtime, h:c.h, t:c.t, ci});
  }));

  const STOP = new Set('the a an and or of to in on for is are was were be with by at as it its this that we i our you they them do did does what who how when where which why about into from over per not no'.split(' '));
  function toks(s){return (s.toLowerCase().match(/[a-z0-9]+/g)||[]).filter(w=>w.length>1&&!STOP.has(w));}
  // light synonym expansion so natural-language questions hit vault jargon
  const SYN={rotate:['rotation'],rotation:['rotate'],token:['tokens','refresh'],tokens:['token'],own:['owns','owner','ownership'],owns:['own','owner','ownership'],owner:['owns','ownership'],cache:['caching'],caching:['cache'],latency:['p95','perf'],speed:['latency','p95'],fast:['latency','perf'],decide:['decision','decisions','decided'],decision:['decided','decisions'],flag:['flags','rollout'],rollout:['flag','flags']};

  function retrieve(query, k){
    k = k||5;
    const qt = toks(query);
    const q = {};
    qt.forEach(w=>{q[w]=(q[w]||0)+1;(SYN[w]||[]).forEach(s=>{q[s]=(q[s]||0)+0.6;});});
    const scored = index.map(ch=>{
      const ct=toks(ch.h+' '+ch.t);
      const tf={}; ct.forEach(w=>tf[w]=(tf[w]||0)+1);
      let dot=0; Object.keys(q).forEach(w=>{if(tf[w])dot+=q[w]*(1+Math.log(tf[w]));});
      const norm=Math.sqrt(ct.length)||1;
      return {ch, raw:dot/norm};
    }).filter(s=>s.raw>0).sort((a,b)=>b.raw-a.raw);

    if(!scored.length) return [];
    // squash into a believable cosine-ish 0.45–0.93 band
    const max=scored[0].raw;
    const top = scored.slice(0,k).map((s,i)=>({
      noteId:s.ch.noteId, path:s.ch.path, title:s.ch.title, mtime:s.ch.mtime,
      h:s.ch.h, chunk:s.ch.t, ci:s.ch.ci,
      score: Math.max(0.44, Math.min(0.93, 0.93*(s.raw/max) - i*0.015))
    }));
    return top;
  }

  function noteById(id){return notes.find(n=>n.id===id);}

  return {notes, index, retrieve, noteById, toks};
})();
