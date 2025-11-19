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
        const isCmd = metaKey || ctrlKey;

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

    indent() {
        const { selectionStart, selectionEnd } = this.textarea;
        const lines = this.getSelectedLines();
        const indentedLines = lines.map(line => '  ' + line.text);
        this.replaceSelectedLines(indentedLines);
        
        const newStart = selectionStart + 2;
        const newEnd = selectionEnd + (indentedLines.length * 2);
        this.setSelection(newStart, newEnd);
    }

    unindent() {
        const { selectionStart, selectionEnd } = this.textarea;
        const lines = this.getSelectedLines();
        
        let totalRemovedChars = 0;
        let removedBeforeCursor = 0;
        
        const unindentedLines = lines.map((line, index) => {
            let removedFromThisLine = 0;
            let newText = line.text;
            
            if (line.text.startsWith('  ')) {
                newText = line.text.slice(2);
                removedFromThisLine = 2;
            } else if (line.text.startsWith(' ')) {
                newText = line.text.slice(1);
                removedFromThisLine = 1;
            } else if (line.text.startsWith('\t')) {
                newText = line.text.slice(1);
                removedFromThisLine = 1;
            }
            
            totalRemovedChars += removedFromThisLine;
            
            const lineStartPos = this.getLineStartPosition(line.index);
            if (lineStartPos < selectionStart) {
                removedBeforeCursor += removedFromThisLine;
            }
            
            return newText;
        });
        
        this.replaceSelectedLines(unindentedLines);
        
        const newStart = Math.max(selectionStart - removedBeforeCursor, 0);
        const newEnd = Math.max(selectionEnd - totalRemovedChars, newStart);
        this.setSelection(newStart, newEnd);
    }

    handleEnter(event) {
        const cursorPos = this.textarea.selectionStart;
        const lines = this.textarea.value.split('\n');
        const currentLineIndex = this.textarea.value.substring(0, cursorPos).split('\n').length - 1;
        const currentLine = lines[currentLineIndex];
        const isLastLine = currentLineIndex == lines.length - 1;
        
        const indentMatch = currentLine.match(/^(\s*)/);
        const indent = indentMatch ? indentMatch[1] : '';
        
        // No text, and it is indented, so we remove one indentation level
        if (currentLine.trim() === '' && indent.length > 0) {
            const newIndent = indent.length >= 2 ? indent.slice(2) : '';
            this.replaceCurrentLine(currentLineIndex, newIndent);
            const lineStartPos = this.getLineStartPosition(currentLineIndex);
            this.setSelection(lineStartPos + newIndent.length, lineStartPos + newIndent.length, true);
            return true;
        }
        
        const listMatch = currentLine.match(/^(\s*)([-*+]|\d+\.)\s(.*)$/);
        const numberedListMatch = currentLine.match(/^(\s*)(\d+)\.\s(.*)$/);
        
        // We check if it is a list, either a bullet or a numbered list
        if (listMatch) {
            // Indentation
            const listIndent = listMatch[1];
            // Type of list (number of bullet character)
            const listMarker = listMatch[2];
            // Content of the list
            const listContent = listMatch[3];
            
            if (listContent.trim() === '') {
                // No content, so we remove an indentation level
                if (listIndent.length >= 2) {
                    // We have one or more indentation levels
                    const newIndent = listIndent.slice(2);
                    const newLine = newIndent + listMarker + ' ';
                    this.replaceCurrentLine(currentLineIndex, newLine);
                    this.setSelection(cursorPos - 2, cursorPos - 2, true);
                } else {
                    // At top level - remove list marker and insert newline
                    this.replaceCurrentLine(currentLineIndex, '');
                    if (isLastLine) {
                        this.insertAtCursor('\n', true);
                    }
                }
                return true;
            }
            
            let newLineContent;
            if (numberedListMatch) {
                const currentNumber = parseInt(numberedListMatch[2]);
                newLineContent = '\n' + listIndent + (currentNumber + 1) + '. ';
            } else {
                newLineContent = '\n' + listIndent + listMarker + ' ';
            }
            
            this.insertAtCursor(newLineContent, true);
            return true;
        }
        
        const newLineContent = '\n' + indent;
        this.insertAtCursor(newLineContent, true);
        return true;
    }

    wrapSelection(before, after) {
        const { selectionStart, selectionEnd } = this.textarea;
        const selectedText = this.textarea.value.substring(selectionStart, selectionEnd);
        
        if (selectedText) {
            const wrappedText = before + selectedText + after;
            this.replaceSelection(wrappedText);
            this.setSelection(selectionStart + before.length, selectionEnd + before.length);
        } else {
            this.insertAtCursor(before + after);
            this.setSelection(selectionStart + before.length, selectionStart + before.length);
        }
    }

    toggleHeading() {
        const { selectionStart, selectionEnd } = this.textarea;
        const lines = this.getSelectedLines();
        let totalCharacterDifference = 0;
        let characterDifferenceBeforeCursor = 0;
        
        const toggledLines = lines.map((line, index) => {
            const headingMatch = line.text.match(/^(#{1,6})\s/);
            let newText;
            let charDifference = 0;
            
            if (headingMatch) {
                newText = line.text.replace(/^#{1,6}\s/, '');
                charDifference = -(headingMatch[0].length);
            } else if (line.text.trim()) {
                newText = '# ' + line.text;
                charDifference = 2;
            } else {
                newText = line.text;
            }
            
            totalCharacterDifference += charDifference;
            
            const lineStartPos = this.getLineStartPosition(line.index);
            if (lineStartPos < selectionStart) {
                characterDifferenceBeforeCursor += charDifference;
            }
            
            return newText;
        });
        
        this.replaceSelectedLines(toggledLines);
        
        const newStart = Math.max(selectionStart + characterDifferenceBeforeCursor, 0);
        const newEnd = Math.max(selectionEnd + totalCharacterDifference, newStart);
        this.setSelection(newStart, newEnd);
    }

    insertHeading(level) {
        const { selectionStart, selectionEnd } = this.textarea;
        const lines = this.getSelectedLines();
        let totalCharacterDifference = 0;
        let characterDifferenceBeforeCursor = 0;
        
        const headingLines = lines.map((line, index) => {
            const headingPrefix = '#'.repeat(level) + ' ';
            let newText;
            let charDifference = 0;
            
            if (line.text.trim()) {
                const cleanLine = line.text.replace(/^#{1,6}\s/, '');
                const existingHeadingMatch = line.text.match(/^#{1,6}\s/);
                const existingHeadingLength = existingHeadingMatch ? existingHeadingMatch[0].length : 0;
                
                newText = headingPrefix + cleanLine;
                charDifference = headingPrefix.length - existingHeadingLength;
            } else {
                newText = line.text;
            }
            
            totalCharacterDifference += charDifference;
            
            const lineStartPos = this.getLineStartPosition(line.index);
            if (lineStartPos < selectionStart) {
                characterDifferenceBeforeCursor += charDifference;
            }
            
            return newText;
        });
        
        this.replaceSelectedLines(headingLines);
        
        const newStart = Math.max(selectionStart + characterDifferenceBeforeCursor, 0);
        const newEnd = Math.max(selectionEnd + totalCharacterDifference, newStart);
        this.setSelection(newStart, newEnd);
    }

    insertLink() {
        const { selectionStart, selectionEnd } = this.textarea;
        const selectedText = this.textarea.value.substring(selectionStart, selectionEnd);
        
        if (selectedText) {
            const linkText = `[${selectedText}](url)`;
            this.replaceSelection(linkText);
            this.setSelection(selectionStart + selectedText.length + 3, selectionStart + selectedText.length + 6);
        } else {
            const linkText = '[text](url)';
            this.insertAtCursor(linkText);
            this.setSelection(selectionStart + 1, selectionStart + 5);
        }
    }

    insertImage() {
        const { selectionStart, selectionEnd } = this.textarea;
        const selectedText = this.textarea.value.substring(selectionStart, selectionEnd);
        
        if (selectedText) {
            const imageText = `![${selectedText}](url)`;
            this.replaceSelection(imageText);
            this.setSelection(selectionStart + selectedText.length + 4, selectionStart + selectedText.length + 7);
        } else {
            const imageText = '![alt text](url)';
            this.insertAtCursor(imageText);
            this.setSelection(selectionStart + 2, selectionStart + 10);
        }
    }

    getLineStartPosition(lineIndex) {
        const lines = this.textarea.value.split('\n');
        let position = 0;
        for (let i = 0; i < lineIndex; i++) {
            position += lines[i].length + 1;
        }
        return position;
    }

    getSelectedLines() {
        const { selectionStart, selectionEnd, value } = this.textarea;
        const beforeSelection = value.substring(0, selectionStart);
        
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
        const oldLine = lines[lineIndex];

        lines[lineIndex] = newContent;
        
        const lineStartPos = this.getLineStartPosition(lineIndex);
        const lineEndPos = lineStartPos + oldLine.length;
        
        this.textarea.focus();
        this.textarea.setSelectionRange(lineStartPos, lineEndPos);
        document.execCommand('insertText', false, newContent);
    }

    replaceSelectedLines(newLines) {
        const { value } = this.textarea;
        const lines = value.split('\n');
        const selectedLines = this.getSelectedLines();
        
        const startIndex = selectedLines[0].index;
        const endIndex = selectedLines[selectedLines.length - 1].index;
        
        const startPos = this.getLineStartPosition(startIndex);
        const endPos = this.getLineStartPosition(endIndex) + lines[endIndex].length;
        
        const newText = newLines.join('\n');
        
        this.textarea.focus();
        this.textarea.setSelectionRange(startPos, endPos);
        document.execCommand('insertText', false, newText);
    }

    replaceSelection(text) {
        const { selectionStart, selectionEnd } = this.textarea;
        
        this.textarea.focus();
        this.textarea.setSelectionRange(selectionStart, selectionEnd);
        document.execCommand('insertText', false, text);
    }

    insertAtCursor(text, immediate = false) {
        const { selectionStart } = this.textarea;
        
        this.textarea.focus();
        document.execCommand('insertText', false, text);
        this.setSelection(selectionStart + text.length, selectionStart + text.length, immediate);
    }

    setSelection(start, end, immediate = false) {
        this.textarea.setSelectionRange(start, end);
        this.textarea.focus();
        this.scrollToCursor(immediate);
    }

    scrollToCursor(immediate = false) {
        const doScroll = () => {
            const { selectionStart } = this.textarea;
            const textBeforeCursor = this.textarea.value.substring(0, selectionStart);
            const lines = textBeforeCursor.split('\n');
            const currentLine = lines.length - 1;
            
            const style = window.getComputedStyle(this.textarea);
            const lineHeight = parseFloat(style.lineHeight);
            const fontSize = parseFloat(style.fontSize);
            const effectiveLineHeight = lineHeight || fontSize * 1.6;
            
            const cursorTop = currentLine * effectiveLineHeight;
            const viewportHeight = this.textarea.clientHeight;
            const scrollTop = this.textarea.scrollTop;
            
            if (cursorTop < scrollTop) {
                this.textarea.scrollTop = cursorTop;
            } else if (cursorTop > scrollTop + viewportHeight - effectiveLineHeight) {
                this.textarea.scrollTop = cursorTop - viewportHeight + effectiveLineHeight * 2;
            }
        };

        if (immediate) {
            doScroll();
        } else {
            setTimeout(doScroll, 0);
        }
    }
}

function enhanceTextareaWithMarkdown(textareaElement) {
    return new MarkdownEditor(textareaElement);
}

