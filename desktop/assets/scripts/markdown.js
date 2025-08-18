class MarkdownEditor {
    constructor(textareaElement) {
        this.textarea = textareaElement;
        this.init();
    }

    init() {
        this.textarea.addEventListener('keydown', (e) => this.handleKeydown(e));
    }

    handleKeydown(event) {
        const { key, metaKey, ctrlKey, shiftKey } = event;
        const isCmd = metaKey || ctrlKey; // Support both Mac (cmd) and Windows/Linux (ctrl)

        // Handle keyboard shortcuts
        if (this.handleShortcuts(event, key, isCmd, shiftKey)) {
            return;
        }

        // Handle special keys
        switch (key) {
            case 'Tab':
                event.preventDefault();
                if (shiftKey) {
                    this.unindent();
                } else {
                    this.indent();
                }
                break;
            case 'Enter':
                if (this.handleEnter(event)) {
                    event.preventDefault();
                }
                break;
        }
    }

    handleShortcuts(event, key, isCmd, shiftKey) {
        if (!isCmd) return false;

        const shortcuts = {
            'b': () => this.wrapSelection('**', '**'), // Bold
            'i': () => this.wrapSelection('_', '_'),   // Italic
            'k': () => this.insertLink(),             // Link
            'd': () => this.wrapSelection('`', '`'),  // Inline code
            'e': () => this.wrapSelection('```\n', '\n```'), // Code block
            'h': () => this.toggleHeading(),         // Heading
            'u': () => this.wrapSelection('<u>', '</u>'), // Underline
            's': () => this.wrapSelection('~~', '~~'), // Strikethrough
            '1': () => this.insertHeading(1),        // H1
            '2': () => this.insertHeading(2),        // H2
            '3': () => this.insertHeading(3),        // H3
        };

        // Special cases with shift modifier
        if (shiftKey) {
            if (key === 'k') {
                event.preventDefault();
                this.insertImage();
                return true;
            }
        }

        if (shortcuts[key]) {
            event.preventDefault();
            shortcuts[key]();
            return true;
        }

        return false;
    }

    // === INDENTATION METHODS ===
    indent() {
        const { selectionStart, selectionEnd } = this.textarea;
        const lines = this.getSelectedLines();
        const indentedLines = lines.map(line => '  ' + line.text);
        this.replaceSelectedLines(indentedLines);
        
        // Adjust cursor position
        const newStart = selectionStart + 2;
        const newEnd = selectionEnd + (indentedLines.length * 2);
        this.setSelection(newStart, newEnd);
    }

    unindent() {
        const { selectionStart, selectionEnd } = this.textarea;
        const lines = this.getSelectedLines();
        const unindentedLines = lines.map(line => {
            if (line.text.startsWith('  ')) {
                return line.text.slice(2);
            } else if (line.text.startsWith(' ')) {
                return line.text.slice(1);
            } else if (line.text.startsWith('\t')) {
                return line.text.slice(1);
            }
            return line.text;
        });
        
        this.replaceSelectedLines(unindentedLines);
        
        // Calculate how many characters were removed
        const removedChars = lines.reduce((acc, line, i) => {
            return acc + (line.text.length - unindentedLines[i].length);
        }, 0);
        
        // Adjust cursor position, ensuring it doesn't go to previous line
        const beforeSelection = this.textarea.value.substring(0, selectionStart);
        const currentLineStart = beforeSelection.lastIndexOf('\n') + 1;
        
        const newStart = Math.max(selectionStart - Math.min(removedChars, 2), currentLineStart);
        const newEnd = Math.max(selectionEnd - removedChars, newStart);
        
        this.setSelection(newStart, newEnd);
    }

    // === ENTER HANDLING ===
    handleEnter(event) {
        const cursorPos = this.textarea.selectionStart;
        const lines = this.textarea.value.split('\n');
        const currentLineIndex = this.textarea.value.substring(0, cursorPos).split('\n').length - 1;
        const currentLine = lines[currentLineIndex];
        
        // Get indentation of current line
        const indentMatch = currentLine.match(/^(\s*)/);
        const indent = indentMatch ? indentMatch[1] : '';
        
        // Check if current line is a list item
        const listMatch = currentLine.match(/^(\s*)([-*+]|\d+\.)\s(.*)$/);
        const numberedListMatch = currentLine.match(/^(\s*)(\d+)\.\s(.*)$/);
        
        if (listMatch) {
            const listIndent = listMatch[1];
            const listMarker = listMatch[2];
            const listContent = listMatch[3];
            
            // Check if the list item is empty (no content after the marker)
            if (listContent.trim() === '') {
                // Empty list item - handle unindentation or removal
                if (listIndent.length >= 2) {
                    // Reduce indentation by one level (2 spaces)
                    const newIndent = listIndent.slice(2);
                    const newLine = newIndent + listMarker + ' ';
                    this.replaceCurrentLine(currentLineIndex, newLine);
                    this.setSelection(cursorPos - 2, cursorPos - 2);
                } else {
                    // At top level - remove list marker entirely
                    this.replaceCurrentLine(currentLineIndex, '');
                    this.setSelection(cursorPos - listMatch[0].length, cursorPos - listMatch[0].length);
                }
                return true;
            }
            
            // Non-empty list item - continue the list
            let newLineContent;
            if (numberedListMatch) {
                // Handle numbered lists
                const currentNumber = parseInt(numberedListMatch[2]);
                newLineContent = '\n' + listIndent + (currentNumber + 1) + '. ';
            } else {
                // Handle bullet lists
                newLineContent = '\n' + listIndent + listMarker + ' ';
            }
            
            this.insertAtCursor(newLineContent);
            return true;
        }
        
        // Not a list item - just preserve indentation
        const newLineContent = '\n' + indent;
        this.insertAtCursor(newLineContent);
        return true;
    }

    // === TEXT WRAPPING METHODS ===
    wrapSelection(before, after) {
        const { selectionStart, selectionEnd } = this.textarea;
        const selectedText = this.textarea.value.substring(selectionStart, selectionEnd);
        
        if (selectedText) {
            const wrappedText = before + selectedText + after;
            this.replaceSelection(wrappedText);
            this.setSelection(selectionStart + before.length, selectionEnd + before.length);
        } else {
            // No selection, insert markers and place cursor between them
            this.insertAtCursor(before + after);
            this.setSelection(selectionStart + before.length, selectionStart + before.length);
        }
    }

    // === HEADING METHODS ===
    toggleHeading() {
        const lines = this.getSelectedLines();
        const toggledLines = lines.map(line => {
            const headingMatch = line.text.match(/^(#{1,6})\s/);
            if (headingMatch) {
                // Remove heading
                return line.text.replace(/^#{1,6}\s/, '');
            } else if (line.text.trim()) {
                // Add heading
                return '# ' + line.text;
            }
            return line.text;
        });
        
        this.replaceSelectedLines(toggledLines);
    }

    insertHeading(level) {
        const lines = this.getSelectedLines();
        const headingLines = lines.map(line => {
            const headingPrefix = '#'.repeat(level) + ' ';
            // Remove existing heading if any
            const cleanLine = line.text.replace(/^#{1,6}\s/, '');
            return cleanLine.trim() ? headingPrefix + cleanLine : line.text;
        });
        
        this.replaceSelectedLines(headingLines);
    }

    // === LINK AND IMAGE METHODS ===
    insertLink() {
        const { selectionStart, selectionEnd } = this.textarea;
        const selectedText = this.textarea.value.substring(selectionStart, selectionEnd);
        
        if (selectedText) {
            const linkText = `[${selectedText}](url)`;
            this.replaceSelection(linkText);
            // Select the "url" part for easy replacement
            this.setSelection(selectionStart + selectedText.length + 3, selectionStart + selectedText.length + 6);
        } else {
            const linkText = '[text](url)';
            this.insertAtCursor(linkText);
            // Select "text" for easy replacement
            this.setSelection(selectionStart + 1, selectionStart + 5);
        }
    }

    insertImage() {
        const { selectionStart, selectionEnd } = this.textarea;
        const selectedText = this.textarea.value.substring(selectionStart, selectionEnd);
        
        if (selectedText) {
            const imageText = `![${selectedText}](url)`;
            this.replaceSelection(imageText);
            // Select the "url" part for easy replacement
            this.setSelection(selectionStart + selectedText.length + 4, selectionStart + selectedText.length + 7);
        } else {
            const imageText = '![alt text](url)';
            this.insertAtCursor(imageText);
            // Select "alt text" for easy replacement
            this.setSelection(selectionStart + 2, selectionStart + 10);
        }
    }

    // === UTILITY METHODS ===
    getSelectedLines() {
        const { selectionStart, selectionEnd, value } = this.textarea;
        const beforeSelection = value.substring(0, selectionStart);
        const afterSelection = value.substring(selectionEnd);
        
        const startLineIndex = beforeSelection.split('\n').length - 1;
        const endLineIndex = value.substring(0, selectionEnd).split('\n').length - 1;
        
        const allLines = value.split('\n');
        const selectedLines = [];
        
        for (let i = startLineIndex; i <= endLineIndex; i++) {
            selectedLines.push({
                index: i,
                text: allLines[i] || ''
            });
        }
        
        return selectedLines;
    }

    replaceCurrentLine(lineIndex, newContent) {
        const lines = this.textarea.value.split('\n');
        lines[lineIndex] = newContent;
        this.textarea.value = lines.join('\n');
    }

    replaceSelectedLines(newLines) {
        const { selectionStart, selectionEnd, value } = this.textarea;
        const lines = value.split('\n');
        const selectedLines = this.getSelectedLines();
        
        // Replace the selected lines with new content
        const startIndex = selectedLines[0].index;
        const endIndex = selectedLines[selectedLines.length - 1].index;
        
        lines.splice(startIndex, endIndex - startIndex + 1, ...newLines);
        
        this.textarea.value = lines.join('\n');
    }

    replaceSelection(text) {
        const { selectionStart, selectionEnd, value } = this.textarea;
        const before = value.substring(0, selectionStart);
        const after = value.substring(selectionEnd);
        
        this.textarea.value = before + text + after;
    }

    insertAtCursor(text) {
        const { selectionStart, value } = this.textarea;
        const before = value.substring(0, selectionStart);
        const after = value.substring(selectionStart);
        
        this.textarea.value = before + text + after;
        this.setSelection(selectionStart + text.length, selectionStart + text.length);
    }

    setSelection(start, end) {
        this.textarea.setSelectionRange(start, end);
        this.textarea.focus();
    }
}

// Function to enhance any textarea with markdown editing capabilities
function enhanceTextareaWithMarkdown(textareaElement) {
    return new MarkdownEditor(textareaElement);
}
