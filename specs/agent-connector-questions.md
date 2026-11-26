# Agent connector: question set

This file is the scoring sheet for the agent connector work. The owner runs it by hand on two or three agent apps. The first run is the baseline, taken from a build of the commit before any change.

## How to run

1. Build the sample vault: `python3 -I tui/tests/agent_questions/build_vault.py <dir>`.
2. Point the agent app at that vault through the Kimun MCP server. Run `kimun workspace reindex` for the vault first.
3. Ask each question in a fresh session. Do not hint at the tools.
4. Fill the scoring table below.

All dates in the vault are the day of the run minus an offset. A range such as "the seven days before today" means from today minus 7 days to today minus 1 day, inclusive, unless the question says otherwise. A good answer states the range that it chose, as real dates.

## Questions

### Time questions

**Q1. What did I do in the seven days before today?**
- Range: the seven days before today (offsets 1 to 7).
- Must read: `journal/` entries for offsets 1 to 7.

**Q2. What did I write in my journal last week?**
- Range: the seven days before today, or the previous calendar week. The answer must name the range.
- Must read: `journal/` entries for offsets 1 to 7.

**Q3. Which meetings did I have in the last 14 days?**
- Range: the 14 days before today (offsets 1 to 14).
- Must read: `meetings/weekly-sync-1.md`, `meetings/weekly-sync-2.md`, `meetings/migration-risk.md`.

**Q4. Which projects are overdue?**
- Range: due date before today.
- Must read: `projects/q4-budget.md`.

**Q5. Which projects are due in the next 30 days?**
- Range: due date from today to today plus 30 days. The latest matching due date is 28 days out, so an inclusive or exclusive end gives the same answer.
- Must read: `projects/mobile-app.md`, `projects/website-redesign.md`, `projects/hiring-plan.md`.

**Q6. What did I decide last month?**
- Range: the 30 days before today (offsets 1 to 30). The answer must name the range.
- Must read: `journal/` entry for offset 12, `meetings/migration-risk.md`.

**Q7. What happened in the 10 days after the Website Redesign kickoff?**
- Range: the kickoff is 14 days before today. The range is offsets 4 to 14.
- Must read: `projects/website-redesign.md`, `journal/` entry for offset 14, `journal/` entry for offset 5.

### Open questions

**Q8. Why is the Mobile App behind schedule?**
- Must read: `projects/mobile-app.md`, `meetings/weekly-sync-1.md`, `journal/` entry for offset 8.

**Q9. What are the risks of the Data Migration?**
- Must read: `projects/data-migration.md`, `meetings/migration-risk.md`, `reading/designing-data-intensive-applications.md`.

**Q10. What do I know about hiring?**
- Must read: `projects/hiring-plan.md`, `meetings/hiring-sync.md`, `journal/` entry for offset 4.

**Q11. What did I read about replication?**
- Must read: `reading/designing-data-intensive-applications.md`, `journal/` entry for offset 10.

### Exact search questions

**Q12. List the notes tagged `#budget`.**
- Must read: `projects/q4-budget.md`, `reference/vendor-list.md`, `journal/` entries for offsets 1, 16 and 24.

**Q13. Which notes have `status` equal to `paused`?**
- Must read: `projects/data-migration.md`.

### Link questions

**Q14. Who is involved in the Q4 Budget, and what do they do?**
- Must read: `projects/q4-budget.md`, `people/alex-rivera.md`, `people/sam-okafor.md`.

**Q15. Which notes link to the Mobile App note?**
- Must read: at least these five, which link to it: `projects/website-redesign.md`, `people/sam-okafor.md`, `people/jordan-lee.md`, `ideas/offline-mode.md`, `ideas/design-tokens.md`. Other notes, such as journal entries and meetings, also link to it.

## Retrieval prompt entries

Each entry runs once with the old prompt and once with the new tools and the `kimun-ask` skill. Record both runs in the table.

**P1. `research_topic`, topic "data migration".**
- Must read: `projects/data-migration.md`, `meetings/migration-risk.md`, `reading/designing-data-intensive-applications.md`, `people/priya-shah.md`.

**P2. `research_note`, note `projects/mobile-app.md`.**
- Must read: `projects/mobile-app.md`, `meetings/weekly-sync-1.md`, `people/sam-okafor.md`, `projects/website-redesign.md`.

**P3. `find_connections`, note `ideas/design-tokens.md`.**
- Must read: `ideas/design-tokens.md`, `projects/website-redesign.md`, `projects/mobile-app.md`.
- A good answer finds connections that the note does not already link, such as `ideas/offline-mode.md`.

**P4. `link_suggestions`, note `ideas/talk-demo.md`.**
- Must read: `ideas/talk-demo.md`, `projects/conference-talk.md`, `people/jordan-lee.md`.
- A good answer suggests notes that are related but not yet linked, and leaves out `projects/conference-talk.md` because the note already links it.

**P5. `brainstorm`, topic "improve the weekly review".**
- Must read: `ideas/weekly-review-template.md`, `reading/building-a-second-brain.md`, `journal/` entry for offset 30.

## Scoring table

Fill one row per question and per run. Scores for "right notes read" and "range stated" are yes, no, or the fraction. "Range stated" is "n/a" for questions without a range.

| Item | Run | Right notes read | Range stated | Tool calls | Tokens |
|------|-----|------------------|--------------|------------|--------|
| Q1 | baseline | | | | |
| Q2 | baseline | | | | |
| Q3 | baseline | | | | |
| Q4 | baseline | | | | |
| Q5 | baseline | | | | |
| Q6 | baseline | | | | |
| Q7 | baseline | | | | |
| Q8 | baseline | | n/a | | |
| Q9 | baseline | | n/a | | |
| Q10 | baseline | | n/a | | |
| Q11 | baseline | | n/a | | |
| Q12 | baseline | | n/a | | |
| Q13 | baseline | | n/a | | |
| Q14 | baseline | | n/a | | |
| Q15 | baseline | | n/a | | |
| P1 | old prompt | | n/a | | |
| P1 | new tools and skill | | n/a | | |
| P2 | old prompt | | n/a | | |
| P2 | new tools and skill | | n/a | | |
| P3 | old prompt | | n/a | | |
| P3 | new tools and skill | | n/a | | |
| P4 | old prompt | | n/a | | |
| P4 | new tools and skill | | n/a | | |
| P5 | old prompt | | n/a | | |
| P5 | new tools and skill | | n/a | | |

For the final run, repeat the Q rows with the run name "final" and compare with the baseline.
