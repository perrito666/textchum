#!/usr/bin/env bash
# A throwaway project and profile to develop against.
#
# Building the editor tells you it compiles. It does not tell you what
# a change looks like against a git repository with a remote, a file
# with uncommitted edits, a nested project, a misspelling and four
# hundred lines to scroll — so this makes one, and opens the working
# copy of the editor on it.
#
#   make playground              build it and open the editor on it
#   make playground KEEP=1       reuse the one that is already there
#   make playground OPEN=0       make it and say where it is, no editor
#
# Everything lives under build/playground: the project on one side, the
# editor's whole profile on the other. The profile is handed over with
# --data-dir, so the configuration, themes, icon packs, session and
# server log of the run are all in there and the real ones are never
# opened.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
BASE="${TEXTCHUM_PLAYGROUND:-$ROOT/build/playground}"
PROJECT="$BASE/project"
PROFILE="$BASE/profile"

if [[ -n "${KEEP:-}" && -d "$PROJECT" ]]; then
    echo "reusing $BASE"
else
    rm -rf "$BASE"
    mkdir -p "$PROJECT" "$PROFILE"

    # ---------------------------------------------------------------
    # A Python project, with a Rust one nested inside it: two languages,
    # two manifests, and a reason for the per-project settings to have
    # something to say.
    # ---------------------------------------------------------------
    mkdir -p "$PROJECT/src" "$PROJECT/tests" "$PROJECT/engine/src"

    cat > "$PROJECT/README.md" <<'EOF'
# Playground

A project that exists to be opened. It has two languages, a remote that
goes nowhere, a file with uncommitted edits, and a deliberate
misspelling: recieve.

- `src/` — the Python side.
- `engine/` — a Rust crate nested inside it.
- `tests/` — what checks the Python side, for the Find References split.
EOF

    cat > "$PROJECT/pyproject.toml" <<'EOF'
[project]
name = "playground"
version = "0.1.0"
requires-python = ">=3.11"

[tool.ruff]
line-length = 88
EOF

    cat > "$PROJECT/src/models.py" <<'EOF'
"""The things the playground keeps track of."""

from dataclasses import dataclass, field


@dataclass
class Item:
    """One item, with a name and a count."""

    name: str
    count: int = 0
    tags: list[str] = field(default_factory=list)

    def label(self) -> str:
        """The item as a line of text."""
        return f"{self.name} x{self.count}"

    def tagged(self, tag: str) -> bool:
        return tag in self.tags


@dataclass
class Basket:
    items: list[Item] = field(default_factory=list)

    def add(self, item: Item) -> None:
        self.items.append(item)

    def total(self) -> int:
        return sum(item.count for item in self.items)

    def named(self, name: str) -> Item | None:
        for item in self.items:
            if item.name == name:
                return item
        return None
EOF

    cat > "$PROJECT/src/app.py" <<'EOF'
"""The entry point, such as it is."""

from models import Basket, Item


def build_basket() -> Basket:
    basket = Basket()
    basket.add(Item(name="apple", count=3, tags=["fruit"]))
    basket.add(Item(name="bread", count=1))
    basket.add(Item(name="cheese", count=2, tags=["dairy"]))
    return basket


def describe(basket: Basket) -> str:
    lines = [item.label() for item in basket.items]
    lines.append(f"{basket.total()} in total")
    return "\n".join(lines)


def main() -> None:
    print(describe(build_basket()))


if __name__ == "__main__":
    main()
EOF

    cat > "$PROJECT/tests/test_app.py" <<'EOF'
from src.app import build_basket, describe
from src.models import Item


def test_basket_counts_everything():
    assert build_basket().total() == 6


def test_an_item_knows_its_label():
    assert Item(name="apple", count=3).label() == "apple x3"


def test_describe_ends_with_the_total():
    assert describe(build_basket()).endswith("6 in total")
EOF

    # Something long enough to scroll, and repetitive enough that the
    # occurrence marks and the outline have plenty to say.
    {
        echo '"""Generated, and long on purpose: something to scroll."""'
        echo
        echo "from models import Item"
        echo
        for n in $(seq 1 60); do
            echo
            echo "def step_$n(item: Item) -> Item:"
            echo "    \"\"\"Step $n of a pipeline that does nothing in particular.\"\"\""
            echo "    item.count += $n"
            echo "    item.tags.append(\"step-$n\")"
            echo "    return item"
        done
        echo
        echo
        echo "PIPELINE = ["
        for n in $(seq 1 60); do echo "    step_$n,"; done
        echo "]"
    } > "$PROJECT/src/pipeline.py"

    cat > "$PROJECT/engine/Cargo.toml" <<'EOF'
[package]
name = "engine"
version = "0.1.0"
edition = "2021"

[dependencies]
EOF

    cat > "$PROJECT/engine/src/lib.rs" <<'EOF'
//! The Rust half of the playground.

/// One item, the same shape the Python side keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub count: u32,
}

impl Item {
    pub fn new(name: &str, count: u32) -> Self {
        Self {
            name: name.to_owned(),
            count,
        }
    }

    /// The item as a line of text.
    pub fn label(&self) -> String {
        format!("{} x{}", self.name, self.count)
    }
}

/// Everything in the basket, counted.
pub fn total(items: &[Item]) -> u32 {
    items.iter().map(|item| item.count).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_basket_counts_everything() {
        let items = vec![Item::new("apple", 3), Item::new("bread", 1)];
        assert_eq!(total(&items), 4);
    }
}
EOF

    cat > "$PROJECT/engine/src/main.rs" <<'EOF'
use engine::{total, Item};

fn main() {
    let items = vec![Item::new("apple", 3), Item::new("cheese", 2)];
    for item in &items {
        println!("{}", item.label());
    }
    println!("{} in total", total(&items));
}
EOF

    cat > "$PROJECT/.gitignore" <<'EOF'
__pycache__/
target/
*.pyc
EOF

    cat > "$PROJECT/notes.md" <<'EOF'
# Notes

Prose, for the spell pass to have something to mark: we recieve the
items, sort them, and seperate the ones that are tagged.

- [ ] Nothing here is real.
- [ ] The remote goes nowhere.
EOF

    # ---------------------------------------------------------------
    # History, so blame and the change gutter have something to show:
    # several commits, by two authors, on different days.
    # ---------------------------------------------------------------
    git -C "$PROJECT" init -q -b main
    git -C "$PROJECT" config user.name "Ada Playground"
    git -C "$PROJECT" config user.email "ada@example.invalid"
    git -C "$PROJECT" config commit.gpgsign false

    commit() {
        GIT_AUTHOR_DATE="$1" GIT_COMMITTER_DATE="$1" \
            GIT_AUTHOR_NAME="$2" GIT_COMMITTER_NAME="$2" \
            GIT_AUTHOR_EMAIL="$3" GIT_COMMITTER_EMAIL="$3" \
            git -C "$PROJECT" commit -q -m "$4"
    }

    git -C "$PROJECT" add README.md pyproject.toml .gitignore
    commit "2026-01-04T09:12:00" "Ada Playground" "ada@example.invalid" \
        "Start the playground"

    git -C "$PROJECT" add src/models.py
    commit "2026-02-11T14:03:00" "Ada Playground" "ada@example.invalid" \
        "Add the item and basket types"

    git -C "$PROJECT" add src/app.py tests/test_app.py
    commit "2026-03-02T11:47:00" "Grace Playground" "grace@example.invalid" \
        "Describe a basket, and check that it counts"

    git -C "$PROJECT" add engine notes.md src/pipeline.py
    commit "2026-05-19T16:28:00" "Grace Playground" "grace@example.invalid" \
        "Add the Rust engine, the pipeline and some prose"

    # A remote that goes nowhere: Copy Forge URL needs one, and nothing
    # here should ever be pushed.
    git -C "$PROJECT" remote add origin \
        https://github.com/textchum-playground/playground.git

    # ---------------------------------------------------------------
    # And the state a working copy is usually in: lines changed, lines
    # added, lines gone, something staged, something untracked.
    # ---------------------------------------------------------------
    # Modified and added lines, uncommitted: the gutter's blue and
    # green. Line by line rather than sed: the text has quotes and
    # braces in it, and an escaping mistake here is a puzzle later.
    label_old='        return f"{self.name} x{self.count}"'
    while IFS= read -r line; do
        if [[ "$line" == "$label_old" ]]; then
            printf '%s\n' '        suffix = " (" + ", ".join(self.tags) + ")" if self.tags else ""'
            printf '%s\n' '        return f"{self.name} x{self.count}{suffix}"'
        else
            printf '%s\n' "$line"
        fi
    done < "$PROJECT/src/models.py" > "$PROJECT/src/models.py.new"
    mv "$PROJECT/src/models.py.new" "$PROJECT/src/models.py"

    cat >> "$PROJECT/src/models.py" <<'EOF'


@dataclass
class Order:
    """Uncommitted: new lines for the gutter to mark."""

    basket: Basket
    placed: bool = False
EOF

    # Lines gone, uncommitted: the gutter's wedge.
    grep -v cheese "$PROJECT/src/app.py" > "$PROJECT/src/app.py.new"
    mv "$PROJECT/src/app.py.new" "$PROJECT/src/app.py"

    # Staged but not committed, and untracked.
    printf '\n# Staged, not committed.\nEXTRA = 1\n' >> "$PROJECT/tests/test_app.py"
    git -C "$PROJECT" add tests/test_app.py
    cat > "$PROJECT/scratch.py" <<'EOF'
# Untracked: no history, so blame has nothing and the gutter stays
# quiet. There is also a syntax error, for whichever server is
# installed to complain about.
def broken(
    print("unclosed")
EOF

    # ---------------------------------------------------------------
    # The profile: empty except for what makes the per-project screens
    # worth looking at.
    # ---------------------------------------------------------------
    cat > "$PROFILE/config.json" <<EOF
{
  "workspace": {
    "manifest_projects": true,
    "projects": {
      "$PROJECT/engine": {
        "ctags_fallback": true,
        "editor": { "tab_width": 4 }
      }
    }
  }
}
EOF
    echo "made $BASE"
fi

echo
echo "  project: $PROJECT"
echo "  profile: $PROFILE   (config, themes, icons, session, lsp.log)"
echo "  remote:  $(git -C "$PROJECT" remote get-url origin)"
echo

if [[ "${OPEN:-1}" == "0" ]]; then
    exit 0
fi

case "$(uname -s)" in
    Darwin)
        make -C "$ROOT" build
        exec "$ROOT/macos/.build/debug/Textchum" --data-dir "$PROFILE" \
            "$PROJECT/src/models.py" "$PROJECT/engine/src/lib.rs" \
            "$PROJECT/notes.md"
        ;;
    *)
        cargo build --manifest-path "$ROOT/linux/Cargo.toml"
        exec "$ROOT/linux/target/debug/textchum-gtk" --data-dir "$PROFILE" \
            "$PROJECT/src/models.py" "$PROJECT/engine/src/lib.rs" \
            "$PROJECT/notes.md"
        ;;
esac
