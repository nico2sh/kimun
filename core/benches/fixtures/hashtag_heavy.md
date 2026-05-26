# Tagged dump

A note with many #hashtag occurrences across the body to exercise the label scanner and the new word-boundary guards.

Mixed labels: #urgent #review #beta #infra #design #qa #release #blocked #stale #archived

Inline mid-prose: We finished #task-1, started #task-2, and #task-3 is waiting on #upstream. The #frontend team owns #component-a, while #backend owns #component-b and #component-c.

Edge cases that should NOT be labels per the new rule:
- ##draft (header territory)
- ###section (deeper header)
- #tag#more (adjacent)
- hello#world (mid-word)
- code: `#inside-backticks`
- link: [docs](page.md#section)

Real labels surrounded by punctuation: ,#after-comma. ;#after-semi. (#in-parens) "#in-quotes"

A run of unique tags to fill the index:
#alpha #bravo #charlie #delta #echo #foxtrot #golf #hotel #india #juliet
#kilo #lima #mike #november #oscar #papa #quebec #romeo #sierra #tango
#uniform #victor #whiskey #xray #yankee #zulu

Repeat tags: #urgent #urgent #urgent #urgent #urgent
More repeats: #review #review #review #review #review

Mixed with wikilinks: see [[topic-a]] for #context and [[topic-b]] for #background.

Trailing line with one last tag: #end
