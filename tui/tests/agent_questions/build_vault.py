#!/usr/bin/env python3
"""Build the sample vault for the agent question set.

Usage: python3 -I tui/tests/agent_questions/build_vault.py <dir>

Every date is the day of the run minus an offset, so the questions can
state their ranges as offsets. Standard library only.
"""
import re
import sys
from datetime import date, timedelta
from pathlib import Path

TODAY = date.today()


def d(offset):
    """The date `offset` days before today, as YYYY-MM-DD."""
    return (TODAY - timedelta(days=offset)).isoformat()


def due(offset):
    """The date `offset` days after today (negative offset means the past)."""
    return (TODAY + timedelta(days=offset)).isoformat()


def toml(**props):
    lines = ["+++"]
    for k, v in props.items():
        if isinstance(v, list):
            lines.append(f"{k} = [{', '.join(repr(x).replace(chr(39), chr(34)) for x in v)}]")
        elif isinstance(v, (int, float)):
            lines.append(f"{k} = {v}")
        elif len(v) == 10 and v[4] == "-" and v[7] == "-":
            lines.append(f"{k} = {v}")
        else:
            lines.append(f'{k} = "{v}"')
    lines.append("+++")
    return "\n".join(lines) + "\n\n"


def yaml(**props):
    lines = ["---"]
    for k, v in props.items():
        if isinstance(v, list):
            lines.append(f"{k}: [{', '.join(v)}]")
        else:
            lines.append(f"{k}: {v}")
    lines.append("---")
    return "\n".join(lines) + "\n\n"


NOTES = {}


def note(path, text):
    NOTES[path] = text


# Journal: the last 45 days. Offsets 0..44. Most days are routine, a few
# carry events that the questions ask about.
EVENTS = {
    0: "Started the day by reviewing the [[website-redesign]] status. #project",
    1: "Call with [[alex-rivera]] about the [[q4-budget]]. He wants the vendor costs cut. #budget",
    2: "Standup ran long. We agreed to move the [[mobile-app]] beta by one week. #project",
    3: "Wrote the first draft of the [[conference-talk]] outline. #writing",
    4: "Interviewed a backend candidate for the [[hiring-plan]]. Strong on databases. #hiring",
    5: "Lunch with [[priya-shah]]. She suggested we look at the [[data-migration]] risks. #migration",
    6: "Quiet day. Cleaned up the inbox and filed old notes. #admin",
    8: "Sprint review for the [[mobile-app]]. Login flow is done, push alerts are late. #project",
    10: "Read [[designing-data-intensive-applications]] chapter 5 on replication. #reading",
    12: "Decided to pause the [[data-migration]] until the backups are tested. #migration #decision",
    14: "The [[website-redesign]] kickoff is today. The new header is approved. #project",
    16: "Budget meeting went badly. [[sam-okafor]] asked for a full cost table. #budget",
    19: "Spent the morning on the [[hiring-plan]]. We need two more engineers. #hiring",
    21: "Sick day. No work done.",
    24: "Drafted the invoice policy for the [[q4-budget]]. #budget",
    27: "Tried a new note taking method after reading [[building-a-second-brain]]. #reading #idea",
    30: "Monthly review. The [[website-redesign]] is on track, the [[mobile-app]] is behind. #review",
    33: "Met [[jordan-lee]] about the conference. The talk is accepted. #writing",
    36: "Moved the old servers into the backup plan for the [[data-migration]]. #migration",
    40: "Planning day. Listed the goals for the quarter. #planning",
    44: "First entry of the period. Set up the new vault. #admin",
}

ROUTINE = [
    "Routine day. Answered email and went through the task list.",
    "Focused work on the current sprint. No blockers.",
    "Short day. Two meetings and some reading.",
    "Reviewed pull requests and updated the board.",
    "Worked from home. Wrote a few notes and tidied the backlog.",
]

for off in range(45):
    day = d(off)
    body = EVENTS.get(off, ROUTINE[off % len(ROUTINE)])
    note(
        f"journal/{day}.md",
        f"# {day}\n\n## Morning\n{body}\n\n## Tasks\n- [ ] Review the task list\n- [x] Clear the inbox\n",
    )

# Projects: TOML frontmatter with created and due dates.
note("projects/website-redesign.md", toml(title="Website Redesign", status="in progress", priority=2, created=d(14), due=due(20), tags=["project", "web"]) +
     "# Website Redesign\n\nThe goal is a new site by the end of the quarter. The owner is [[priya-shah]].\n\n## Scope\nNew header, new landing page, faster pages. See [[mobile-app]] for the shared design system.\n\n## Risks\nThe copy is not ready. The photos are late.\n\n#project #web\n")
note("projects/mobile-app.md", toml(title="Mobile App", status="at risk", priority=1, created=d(60), due=due(5), tags=["project", "mobile"]) +
     "# Mobile App\n\nThe beta slipped by one week. The login flow is done. Push alerts are late.\n\n## Team\n[[sam-okafor]] leads the work. [[jordan-lee]] tests it.\n\nSee [[website-redesign]] for the design system.\n\n#project #mobile\n")
note("projects/q4-budget.md", toml(title="Q4 Budget", status="open", priority=1, created=d(30), due=due(-3), tags=["budget", "finance"]) +
     "# Q4 Budget\n\nThe budget needs a full cost table. [[alex-rivera]] wants the vendor costs cut. [[sam-okafor]] reviews the final numbers.\n\n## Vendors\nThree vendors, two over budget.\n\n#budget #finance\n")
note("projects/hiring-plan.md", toml(title="Hiring Plan", status="open", priority=2, created=d(19), due=due(28), tags=["hiring"]) +
     "# Hiring Plan\n\nWe need two more engineers. One backend, one mobile.\n\n## Process\nPhone screen, then a technical interview, then a team meeting. [[alex-rivera]] signs the offers.\n\n#hiring\n")
note("projects/data-migration.md", yaml(title="Data Migration", status="paused", priority="3", created=d(40), due=due(60), tags=["migration", "ops"]) +
     "# Data Migration\n\nMove the old database to the new cluster. Paused until the backups are tested.\n\n## Risks\nLong downtime. See [[priya-shah]] for the rollback plan.\n\n#migration #ops\n")
note("projects/conference-talk.md", yaml(title="Conference Talk", status="in progress", priority="3", created=d(33), due=due(45), tags=["writing"]) +
     "# Conference Talk\n\nThe talk is accepted. Outline in progress. [[jordan-lee]] is the organizer contact.\n\n## Outline\n1. The problem\n2. The method\n3. Results\n\n#writing\n")
note("projects/old-intranet.md", toml(title="Old Intranet", status="done", priority=3, created=d(200), tags=["archived"]) +
     "# Old Intranet\n\nThis project is finished. It has no due date.\n\n#archived\n")

# People.
note("people/alex-rivera.md", "# Alex Rivera\n\nHead of finance. Decides on the [[q4-budget]] and signs offers for the [[hiring-plan]].\n\n#person\n")
note("people/priya-shah.md", "# Priya Shah\n\nEngineering manager. Owns the [[website-redesign]] and the rollback plan for the [[data-migration]].\n\n#person\n")
note("people/sam-okafor.md", "# Sam Okafor\n\nTech lead of the [[mobile-app]]. Reviews numbers for the [[q4-budget]].\n\n#person\n")
note("people/jordan-lee.md", "# Jordan Lee\n\nConference organizer contact and tester of the [[mobile-app]]. Related to the [[conference-talk]].\n\n#person\n")

# Meetings: a `date` property each.
meet = [
    ("weekly-sync-1", "Weekly Sync", 3, "Agreed to move the beta. Owner: Sam. Topics: [[mobile-app]]."),
    ("weekly-sync-2", "Weekly Sync", 10, "Reviewed the header design. Topics: [[website-redesign]]."),
    ("budget-review", "Budget Review", 16, "The table was missing vendor costs. Topics: [[q4-budget]]."),
    ("hiring-sync", "Hiring Sync", 19, "Two roles open. Topics: [[hiring-plan]]."),
    ("migration-risk", "Migration Risk Review", 12, "Pause until backups are tested. Topics: [[data-migration]]."),
    ("quarter-planning", "Quarter Planning", 40, "Goals for the quarter. Topics: [[website-redesign]], [[mobile-app]]."),
]
for slug, title, off, body in meet:
    note(f"meetings/{slug}.md", toml(title=title, date=d(off), tags=["meeting"]) + f"# {title}\n\n{body}\n\n#meeting\n")

# Ideas.
note("ideas/offline-mode.md", "# Offline Mode\n\nThe app could work without a network. It would sync later. Related to the [[mobile-app]].\n\n#idea\n")
note("ideas/weekly-review-template.md", "# Weekly Review Template\n\nA fixed set of headings for the weekly review. Inspired by [[building-a-second-brain]].\n\n#idea\n")
note("ideas/design-tokens.md", "# Design Tokens\n\nOne source of colors and spacing for the web and the mobile app. Helps the [[website-redesign]] and the [[mobile-app]].\n\n#idea #design\n")
note("ideas/talk-demo.md", "# Talk Demo\n\nA live demo for the [[conference-talk]]. Needs a fallback video.\n\n#idea #writing\n")

# Reading notes.
note("reading/designing-data-intensive-applications.md", "# Designing Data-Intensive Applications\n\nChapter 5 covers replication: leaders, followers, and lag. Useful for the [[data-migration]].\n\n## Key points\n- Replication lag causes stale reads.\n- Failover needs a tested plan.\n\n#reading #databases\n")
note("reading/building-a-second-brain.md", "# Building a Second Brain\n\nA method to capture, organize, distill and express notes. Led to the [[weekly-review-template]].\n\n#reading #method\n")
note("reading/the-pragmatic-programmer.md", "# The Pragmatic Programmer\n\nTips on craft. Remember the broken windows idea.\n\n#reading\n")
note("reading/atomic-habits.md", "# Atomic Habits\n\nSmall changes add up. Link habits to existing routines.\n\n#reading #habits\n")

# Reference.
note("reference/glossary.md", "# Glossary\n\n- Beta: the first public test build.\n- Cutover: the moment of the switch in the [[data-migration]].\n\n#reference\n")
note("reference/vendor-list.md", "# Vendor List\n\nThree vendors for hosting, design and testing. Costs are in the [[q4-budget]].\n\n#reference #budget\n")
note("reference/orphan-note.md", "# Orphan Note\n\nThis note has no links in or out. It talks about the history of the office plants.\n\n#misc\n")


def check_links():
    """Exit non-zero unless every [[target]] matches exactly one written file.

    Core lowercases the target and matches it as a suffix of the file path
    without the extension.
    """
    stems = [p[: -len(".md")].lower() for p in NOTES]
    bad = []
    for path, text in NOTES.items():
        for target in re.findall(r"\[\[([^\]|#]+)", text):
            t = target.strip().lower()
            hits = [x for x in stems if x == t or x.endswith("/" + t)]
            if len(hits) != 1:
                bad.append(f"{path}: [[{target}]] matches {len(hits)} notes")
    if bad:
        sys.exit("unresolved links:\n" + "\n".join(bad))


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: build_vault.py <dir>")
    root = Path(sys.argv[1])
    for rel, text in NOTES.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
    check_links()
    print(f"wrote {len(NOTES)} notes to {root} (today is {TODAY.isoformat()})")


if __name__ == "__main__":
    main()
