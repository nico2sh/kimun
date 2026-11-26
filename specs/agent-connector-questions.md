# Agent connector: question set

This file is the scoring sheet for the agent connector work. The owner runs it by hand on two or three agent apps. The first run is the baseline, taken from a build of the commit before any change.

## Summary of results

Date of the runs: 2026-11-26. Each question ran once per build, so one run is thin evidence. Tokens were not measured. The baseline is a build of `main`. The final build is this branch. Some final runs came before the last guidance changes, and the notes below the table say which.

Correctness, notes read:

- Both builds answered Q2, Q3, Q4, Q5, Q10, Q11, Q12, Q13 and Q15 correctly.
- The baseline missed notes on Q8 (2 of 3), Q9 (2 of 3) and Q14 (1 of 3). The final build read all of them.
- The baseline answered Q1 from git history and read no notes. This run was in the repository folder and was not rerun in `~/kimun-eval`, so treat it as a weak data point.
- Both builds missed the meeting note on Q6b (1 of 2).
- On Q7 the baseline read all three notes. The final build read two and did not open the project note, also after the rule about topic searches. Its second run checked the project note's backlinks and said that a date range does not search note text.
- The baseline did better on Q6. It found the decision of 2026-11-14 and said that it falls in this month. The final build used a hard date range for October and missed it.

Tool calls, over the 14 questions with valid runs on both sides (Q2 to Q7, Q6b and Q8 to Q14):

- The baseline took 93 calls and the final build took 58, about 38 percent fewer.
- The final build was cheaper on Q2 (1 against 8), Q3 (3 against 15), Q5 (1 against 4), Q6 (8 against 14) and Q7 (8 against 18).
- The baseline was cheaper on Q4 (2 against 3), Q6b (8 against 13) and Q14 (1 against 5). The extra calls on Q14 bought a complete answer.
- The Q15 final call count is not valid and was not rerun.

How the answers read:

- The final build put a note path next to nearly every claim. The baseline cited paths on only some answers.
- The final build said which parts came from its own reasoning and what it had read in full or as a snippet. The baseline made unlabeled inferences on Q9 and Q10.

Retrieval prompts:

- P1: both arms found all four notes. The old prompt used 0 calls and the new tools used 6.
- P4: the old prompt said that no related notes exist, and the agent believed it (1 of 3). The new tools found the right link (3 of 3, 9 calls).
- P5: the old arm read 2 of 3 notes in 8 calls. The new arm read 3 of 3 in 20 calls.
- Four of the five prompts could not take their required argument from Claude Code. The server lists `max_results` first and sends numbers as text.
- Decision: the five retrieval prompts were removed. The skill keeps their recipes.

Risks that remain:

- A hard date range can hide a note just outside it (Q6). The guidance now asks the agent to name the other reading, and to say when it did not search it.
- The new build costs more calls on some questions (Q4, Q6b, Q14, and P5).
- The `vault_info` call and the longer instructions have a token cost that nobody measured.

Changes made because of the runs:

- A rule for notes that keep their dates in different places (Q3).
- A rule to name the other reading of a phrase and to widen a thin answer (Q6).
- A rule to put the path next to each claim.
- A rule that a date range finds only notes with that date key, so a topic in a period also needs a topic search (Q6b and Q7).
- A fix for a control character in `semantic_search` titles.
- Fixes to the sheet and the fixture (Q2, Q6 and Q7).
- The removal of the five prompts.

## How to run

1. Build the sample vault: `python3 -I tui/tests/agent_questions/build_vault.py <dir>`.
2. Point the agent app at that vault through the Kimun MCP server. Run `kimun workspace reindex` for the vault first.
3. Ask each question in a fresh session. Start every session from an empty folder outside any git repository, for example `~/kimun-eval`. In a repository folder the agent can answer from git history and never touch the notes. Do not hint at the tools and do not name the notes.
4. Fill the scoring table below.

All dates in the vault are the day of the run minus an offset. A range such as "the seven days before today" runs from today minus 7 days to today minus 1 day. Both ends are inclusive, unless the question says otherwise. A good answer states the range that it chose, as real dates.

## Questions

### Time questions

**Q1. What did I do in the seven days before today?**
- Range: the seven days before today (offsets 1 to 7).
- Must read: `journal/` entries for offsets 1 to 7.

**Q2. What did I write in my journal last week?**
- Range: the seven days before today, or the previous calendar week (Monday to Sunday, the week before the current one). The answer must name the range.
- Must read: the `journal/` entries that fall in the range that the answer states. For the seven days before today, these are offsets 1 to 7. For the previous calendar week, these are the entries from that Monday to that Sunday. The offsets depend on the weekday of the run day, so check the dates in the answer. Score either choice as correct. The notes must match the stated range.

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
- Range: the 30 days before today (offsets 1 to 30), or the previous calendar month. The answer must name the range.
- Must read: for the 30 days, the `journal/` entry for offset 12 and `meetings/migration-risk.md`. These two carry the decision tagged `#decision`. For the previous calendar month, the fixture can hold no decision, because the decision sits at offset 12. Then a good answer says that it found none, and it does not invent one. A good answer for either choice also names the other reading.
- Scoring note: under the calendar reading, an answer can say that it did not search the other reading. It must not state what the other reading holds unless it searched it.

**Q6b. What did I decide in the last 30 days?**
- Range: the 30 days before today (offsets 1 to 30). The answer must name the range. This wording has one reading, so it compares a baseline run with a final run on retrieval alone. Q6 tests how the agent handles the two readings.
- Must read: the `journal/` entry for offset 12 and `meetings/migration-risk.md`. These two carry the decision to pause the data migration. The journal entry is tagged `#decision`.

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

The five prompts below were removed after this comparison, so the old arm runs on a build of `main`. To run an old prompt in Claude Code, type its slash command. Claude Code fills the arguments by position, in the order that the server declares them. The server puts `max_results` first and the required argument second, so `/mcp__kimun__research_topic data migration` fails with "expected u32". Type the number first, for example `/mcp__kimun__research_topic 10 "data migration"`. The same order applies to `brainstorm`, `link_suggestions` and `research_note`. Hide the `kimun-ask` skill for the old-prompt run, and put it back for the new-tools run.

P2 and P3 were not run. The prompts were removed after P1, P4 and P5, so their rows are gone from the table.

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
| Q1 | baseline | 0/7 (answered from git history, repo folder, not rerun) | yes (about commits) | 0 kimun, 2 shell | not measured |
| Q1 | final | 7/7 (from result list, not rerun) | yes | not recorded | not measured |
| Q2 | baseline | 7/7 (calendar week, no paths cited) | yes | 8 | not measured |
| Q2 | final | 7/7 (calendar week, shared session, not rerun) | yes | 1 | not measured |
| Q3 | baseline | 3/3 (no paths cited) | yes | 15 | not measured |
| Q3 | final | 3/3 (rerun with the latest guidance) | yes | 3 | not measured |
| Q4 | baseline | 1/1 | yes | 2 | not measured |
| Q4 | final | 1/1 | yes | 3 | not measured |
| Q5 | baseline | 3/3 (no paths cited) | yes | 4 | not measured |
| Q5 | final | 3/3 | yes | 1 | not measured |
| Q6 | baseline | 1/2 vs 30-day set (decision found outside its month, meeting note not cited) | month named, no dates | 14 | not measured |
| Q6 | final | 0/2 vs 30-day set (calendar reading, accepted, not rerun) | yes | 8 | not measured |
| Q6b | baseline | 1/2 (decision found with the `#decision` tag, meeting note not cited, no paths) | 30 days named, no dates | 8 | not measured |
| Q6b | final | 1/2 (decision found, meeting note not cited, not rerun) | yes | 13 | not measured |
| Q7 | baseline | 3/3 | yes (2026-11-13 to 2026-11-22) | 18 kimun, 1 shell | not measured |
| Q7 | final | 2/3 (project note's backlinks checked, note not cited; rerun after the topic rule) | yes (2026-11-13 to 2026-11-22) | 8 | not measured |
| Q8 | baseline | 2/3 (weekly sync note not read) | n/a | 5 kimun, 2 shell | not measured |
| Q8 | final | 3/3 | n/a | 3 | not measured |
| Q9 | baseline | 2/3 (reading note not read) | n/a | 7 | not measured |
| Q9 | final | 3/3 | n/a | 5 | not measured |
| Q10 | baseline | 3/3 (no paths cited) | n/a | 6 | not measured |
| Q10 | final | 3/3 | n/a | 4 | not measured |
| Q11 | baseline | 2/2 (no paths cited) | n/a | 3 | not measured |
| Q11 | final | 2/2 | n/a | 2 | not measured |
| Q12 | baseline | 5/5 | n/a | 1 | not measured |
| Q12 | final | 5/5 | n/a | 1 | not measured |
| Q13 | baseline | 1/1 | n/a | 1 | not measured |
| Q13 | final | 1/1 | n/a | 1 | not measured |
| Q14 | baseline | 1/3 (people notes not read) | n/a | 1 | not measured |
| Q14 | final | 3/3 | n/a | 5 | not measured |
| Q15 | baseline | 5/5 (all 10 links right) | n/a | 1 | not measured |
| Q15 | final | 5/5 (all 10 links right, reused context, not rerun) | n/a | 0 (reused context) | not measured |
| P1 | old prompt | 4/4 (all four expected notes, plus glossary and three journal entries) | n/a | 0 (the prompt text carried the notes) | not measured |
| P1 | new tools and skill | 4/4 (project, meeting and Priya read in full, reading note as a snippet) | n/a | 6 | not measured |
| P4 | old prompt | 1/3 (only the note itself) | n/a | 0 | not measured |
| P4 | new tools and skill | 3/3 (note, talk note and Jordan Lee read in full) | n/a | 9 | not measured |
| P5 | old prompt | 2/3 (template and reading note read, monthly review entry not read) | n/a | 8 | not measured |
| P5 | new tools and skill | 3/3 (template, book note and monthly review entry read) | n/a | 20 | not measured (counter showed about 2.8K, meaning unclear) |

### Notes on the final rows

The final rows record the runs of 2026-11-26 on the new build, from the answers that the owner pasted. Tokens were not measured. These facts change how to read some rows.

- Q1 and Q2 ran in one session, so Q2 had a shared context. The call count for Q1 was not recorded. The owner decided not to rerun them.
- Q3 first failed with 0 of 3 notes and 2 calls. It passed after the guidance change on dates. The rerun with the latest guidance passed with 3 of 3 notes in 3 calls. It named the other reading and said that it did not search it. It also said that a journal search for the range found nothing, but the journal entry of 2026-11-19 mentions two meetings.
- Q6 ran three times with different guidance text. The row shows the last run. The owner decided not to rerun it. Under the calendar-month reading the answer is accepted, and the sheet expects no decision. The agent searched only that reading and said so.
- Q6b is the 30-day wording. Its first run found the decision tagged `#decision` and did not cite the meeting note.
- Q7 was an invalid question because of a fixture fault, now fixed. The rerun on the rebuilt vault read the kickoff entry and the entry of 2026-11-21, and it did not read the project note. The first rerun took 6 calls and wrote one date wrong in the other reading (2026-12-12 for 2026-11-12). After the rule about topic searches, a second rerun took 8 calls (baseline: 18). It searched the journal, date and created keys, checked the backlinks of the project note, and said that a date range does not search note text. It still did not cite the project note itself.
- Q15 reused the answer from earlier in the session. The ten links are correct, but the call count is not valid. The owner decided not to rerun it, because the baseline shows that the old tool already handles it in one call.


### Notes on the baseline rows

The baseline rows record runs on a build of `main`, started with `--strict-mcp-config` and `--disable-slash-commands`.

- Q1 ran in the repository folder. The agent read the git history and never called `kimun`. Rerun it from `~/kimun-eval` and compare that run.
- Q2 ran in the repository folder and used the notes. It took 8 calls where the final run took 1, and it cited no paths. It named the calendar week and the other reading.
- Q3 found all three meeting notes in 15 calls where the final run took 3. It also listed five meeting-like journal events and named three older meeting notes, so its answer is wider than the final one. It cited no paths.
- Q4 found the one overdue project in 2 calls where the final run took 3. The old search tool already took `%due<date` in its query, so the agent needed no new tool. The answer named the status check and the limit for notes with no due date.
- Q5 found all three projects in 4 calls where the final run took 1. Both gave the right range and due dates. The baseline cited no paths and said that the search left out finished projects and projects with no due date.
- Q6 read "last month" as October and found no October decision. It still found the decision of 2026-11-14 with its journal path, and said that it falls in this month. It searched words across the whole vault, not a date range, so the decision showed up. It did not name the other reading and it took 14 calls.
- Q7 ran on the rebuilt vault, so the kickoff is on 2026-11-12. The agent read the project note, the kickoff entry and the entry for 2026-11-21, and gave the right range. It took 18 calls and one shell command. The old first run of this question on the new build is invalid, so no final row exists yet.
- Q6b found the decision of 2026-11-14 and said that it is the only note with the `#decision` tag. It missed the meeting note, as the final run did, and it cited no paths. It took 8 calls where the final run took 13. It named the range as "the last 30 days" and gave no dates.
- Q8 read the project note and the entry for 2026-11-18, and said that the cause is not written down. It did not open `meetings/weekly-sync-1.md`, and it said so. It also ran two shell commands to look at the repository. It took 5 calls where the final run took 3 and read all three notes.
- Q9 read the project note, the migration risk meeting note and Priya's note. It did not read the reading note on replication. It added a reason from general knowledge (untested backups can leave no way to restore data) without saying that the notes do not hold it. It took 7 calls where the final run took 5 and read all three notes.
- Q10 read the three expected notes in 6 calls where the final run took 4. It stated that the budget limits what you can offer, and then said that it had not opened the budget note. The first statement is an unlabeled inference. It labeled the other inference, about the mobile role.
- Q11 read both expected notes in 3 calls where the final run took 2. It explained leaders and followers from general knowledge without saying so. It labeled its link to the data migration as its own reading. It did not check that the pause came before the reading, as the final run did.
- Q12 listed the five notes in 1 call, the same as the final run. It gave the paths and titles only. The final run added a one-line summary for each note and named the limit of a tag search.
- Q13 found the one paused note in 1 call, the same as the final run. The baseline gave the path and title only. The final run added dates and the limit of an exact property search.
- Q14 read only the project note, in 1 call where the final run took 5 and read all three notes. It gave the two names and one fact for each from the project note. It said that it did not open their notes to look for job titles. The question asks what they do, so its answer is incomplete.
- Q15 listed all ten linking notes in 1 call. This is a fresh run, so it is the valid count. The final run reused an earlier answer and needs a fresh run to compare.
- P1 old prompt: the prompt could not be typed in Claude Code (see the note above the prompt entries). Its message came from the baseline server through the protocol and was pasted as the first prompt. The message held all four expected notes and no tool call was needed. The answer followed the four questions in the prompt, listed seven gaps and cited few paths.
- P1 new tools and skill: the same four questions without the search results. It found the four expected notes in 6 calls and cited a path with nearly every claim. It gave seven gaps, and said that the gap list is its own reading and what it read in full or as a snippet. The old prompt took 0 calls and cited few paths.
- P4 old prompt: the message said "No unlinked related notes found in the vault". The agent suggested nothing and asked what to do. It made no tool call. The sheet expects related notes such as `people/jordan-lee.md`.
- P4 new tools and skill: the agent suggested `people/jordan-lee.md` and left out `projects/conference-talk.md`, which the note already links. It rejected four other candidates with reasons. It used 9 calls. It wrote each finding with likelihood, impact, cost and verdict, because the owner's global instructions ask for that in review findings. That format comes from those instructions and not from the connector.
- P5 old prompt: the message held no notes, only the task. The agent searched by itself in 8 calls and wrote ten ideas, each tied to a real note (all notes it named exist). It did not read the monthly review entry of 2026-10-27, which the sheet expects. It found `reading/atomic-habits.md`, which the sheet does not list.
- P5 new tools and skill: the agent read all three expected notes, including the monthly review entry that the old arm missed. It wrote eight ideas, and said that the ideas are its own and that the notes and links are real. It took 20 calls where the old arm took 8, and it sampled many journal days.

For the comparison, fill the baseline rows from a build of `main` and compare each pair of rows.
