# Edge cases

Code span `[[x]]` and code link `[y](y.md)` stay code.

Alias tag [[a|#b]] and invalid [[#tag]] and empties [[|b]] [[]].

A reference [ref][r] and an autolink <https://e.x>.

Embed ![[pic.png|Picture]] and plain [[note|Shown]].

Broken across lines [[target
|Shown]] then text.

A wikilink in an HTML block:

<details>
<summary>More</summary>
See [[hidden]]
</details>

Alias markup [[a|*Emph* name]] and entity [[c|a &amp; b]].

Touching tags [[c]]#t2 and #t3[[d]].

Image alt ![x [[a]] #t](p.png) degrades.

Autolink fragment <https://x.com/#frag> is no tag.

## After the broken link

body

[r]: other.md
