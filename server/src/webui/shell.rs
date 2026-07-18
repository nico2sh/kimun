//! Page shell (nav + layout) and the embedded stylesheet shared by every page.

use maud::{DOCTYPE, Markup, html};

use crate::server_state::AppState;

pub(super) fn shell(state: &AppState, active: &str, title: &str, body: Markup) -> Markup {
    let auth_on = state.config.auth.token.is_some();
    let nav = [
        ("/", "Dashboard"),
        ("/config", "Config"),
        ("/collections", "Collections"),
        ("/jobs", "Jobs"),
        ("/logs", "Logs"),
        ("/query", "Test query"),
    ];
    html! {
        (DOCTYPE)
        html {
            head {
                meta charset="utf-8";
                title { "Kimün RAG — " (title) }
                link rel="icon" type="image/png" href="/assets/img/kimun.png";
                (styles())
            }
            body {
                nav {
                    img .logo src="/assets/img/kimun.png" alt="" width="20" height="20";
                    span .brand { "Kimün RAG" }
                    @for (href, label) in nav {
                        a href=(href) .active[active == href] { (label) }
                    }
                    @if auth_on { a href="/logout" .right { "Sign out" } }
                }
                main { (body) }
            }
        }
    }
}

pub(super) fn styles() -> Markup {
    html! {
        style {
            (maud::PreEscaped(r#"
@font-face{font-family:"Atkinson Hyperlegible Mono";src:url(/assets/fonts/ahm-regular.woff2) format("woff2");font-weight:400;font-display:swap}
@font-face{font-family:"Atkinson Hyperlegible Mono";src:url(/assets/fonts/ahm-bold.woff2) format("woff2");font-weight:700;font-display:swap}
@font-face{font-family:"Inter";src:url(/assets/fonts/inter-regular.woff2) format("woff2");font-weight:400;font-display:swap}
@font-face{font-family:"Inter";src:url(/assets/fonts/inter-semibold.woff2) format("woff2");font-weight:600;font-display:swap}
:root{
  --bg:oklch(20% .008 75);
  --panel:oklch(23.5% .009 75);
  --line:oklch(31% .012 75);
  --fg:oklch(91% .012 85);
  --muted:oklch(67% .018 80);
  --accent:oklch(84% .14 89);
  --link:oklch(76% .07 230);
  --ok:oklch(76% .1 145);
  --err:oklch(74% .12 30);
  --mono:"Atkinson Hyperlegible Mono",ui-monospace,SFMono-Regular,Menlo,monospace;
  --sans:"Inter",system-ui,sans-serif;
  --sp-xs:.25rem;--sp-sm:.5rem;--sp-md:.75rem;--sp-lg:1rem;--sp-xl:1.5rem;--sp-2xl:2rem;--sp-3xl:3rem;
}
*{box-sizing:border-box}
body{margin:0;font:1rem/1.65 var(--sans);color:var(--fg);background:var(--bg)}
a{color:var(--link)}
a:focus-visible,input:focus-visible,select:focus-visible,button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
nav{display:flex;gap:var(--sp-lg);align-items:baseline;padding:var(--sp-md) var(--sp-xl);border-bottom:1px solid var(--line)}
nav .logo{align-self:center;border-radius:4px}
nav .brand{font:700 1rem var(--mono);margin-right:var(--sp-lg)}
nav a{color:var(--muted);text-decoration:none;font:400 .875rem var(--mono)}
nav a:hover{color:var(--fg)}
nav a.active{color:var(--accent)}
nav a.right{margin-left:auto}
main{max-width:880px;margin:var(--sp-3xl) auto;padding:0 var(--sp-xl)}
main.login{max-width:22rem;margin-top:16vh}
main.login .logo-lg{border-radius:8px;margin-bottom:var(--sp-md)}
h1{font:700 1.5625rem/1.3 var(--mono);letter-spacing:-.01em;margin:0 0 var(--sp-xl)}
h2{font:700 1rem/1.4 var(--mono);margin:var(--sp-2xl) 0 var(--sp-md)}
p{max-width:70ch}
.statusline{font:400 .9375rem/1.6 var(--mono);margin:calc(-1*var(--sp-md)) 0 var(--sp-2xl);color:var(--muted)}
.statusline b{color:var(--fg);font-weight:400}
.panel{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:var(--sp-xl)}
section.group{border-top:1px solid var(--line);margin-top:var(--sp-xl);padding-top:var(--sp-lg)}
section.group h2{margin:0 0 var(--sp-md)}
table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}
th,td{text-align:left;padding:var(--sp-sm) var(--sp-lg) var(--sp-sm) 0;border-bottom:1px solid var(--line);vertical-align:top}
th{font:700 .75rem var(--mono);color:var(--muted);text-transform:uppercase;letter-spacing:.08em}
dl{display:grid;grid-template-columns:max-content 1fr;gap:var(--sp-sm) var(--sp-2xl);margin:0}
dt{color:var(--muted);font:400 .875rem/1.7 var(--mono)}
dd{margin:0;font-variant-numeric:tabular-nums}
label{display:block;margin:var(--sp-lg) 0 var(--sp-xs);color:var(--muted);font:400 .8125rem var(--mono)}
input,select{width:100%;padding:var(--sp-sm) var(--sp-md);border:1px solid var(--line);border-radius:6px;background:var(--panel);color:var(--fg);font:400 .875rem var(--mono)}
input::placeholder{color:var(--muted)}
.row{display:flex;gap:var(--sp-lg)}.row>div{flex:1}
.check{display:flex;align-items:center;gap:var(--sp-sm)}.check input{width:auto}
button{margin-top:var(--sp-xl);padding:var(--sp-sm) var(--sp-xl);border:0;border-radius:6px;background:var(--accent);color:oklch(24% .03 85);font:700 .875rem var(--mono);cursor:pointer}
button:hover{background:oklch(88% .13 89)}
button.danger{background:var(--err);color:oklch(20% .05 30)}
button.danger:hover{background:oklch(66% .14 30)}
.flash{padding:var(--sp-md) var(--sp-lg);border-radius:6px;margin:var(--sp-lg) 0;font-size:.9375rem}
.flash.ok{background:oklch(76% .1 145/.12);color:var(--ok)}
.flash.err{background:oklch(74% .12 30/.12);color:var(--err)}
.flash a{color:inherit;text-decoration:underline}
.muted{color:var(--muted)}
.mono{font-family:var(--mono);font-size:.875rem}
.snippet{color:var(--muted);font-size:.875rem;max-width:70ch}
.badge{display:inline-block;padding:.05rem .5rem;border-radius:4px;font:400 .75rem var(--mono);background:var(--panel);border:1px solid var(--line)}
.status{display:inline-flex;align-items:center;gap:.45em;font:400 .875rem var(--mono)}
.status::before{content:"";width:.5em;height:.5em;border-radius:50%;background:var(--muted);flex:none}
.status.processing::before{background:var(--accent);animation:pulse 1.6s ease-out infinite}
.status.completed::before{background:var(--ok)}
.status.failed::before{background:var(--err)}
.live{color:var(--muted);font:400 .75rem var(--mono)}
.live::before{content:"";display:inline-block;width:.45em;height:.45em;border-radius:50%;background:var(--ok);margin-right:.4em;animation:pulse 2s ease-out infinite}
.hit{margin:var(--sp-lg) 0 0;padding-top:var(--sp-lg);border-top:1px solid var(--line)}
.hit .score{color:var(--muted);font:400 .75rem var(--mono);margin-left:var(--sp-sm)}
@keyframes pulse{0%,100%{opacity:1}50%{opacity:.35}}
@media (prefers-reduced-motion:reduce){.status.processing::before,.live::before{animation:none}}
"#))
        }
    }
}
