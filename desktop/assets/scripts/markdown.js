/**
 * Pure JavaScript Markdown Editor for Textarea
 * Replicates textarea-markdown-editor functionality with keyboard shortcuts only
 * No buttons, no visual enhancements - just shortcuts for formatting
 */

(function () {
    'use strict';

    class Cursor {
        constructor(textarea) {
            this.textarea = textarea;
            this.MARKER = '::MARKER::';
        }

        get position() {
            const textarea = this.textarea;
            const start = textarea.selectionStart;
            const value = textarea.value;
            const beforeCursor = value.substring(0, start);
            const lineNumber = beforeCursor.split('\n').length;

            return {
                cursorAt: start,
                lineNumber: lineNumber
            };
        }

        get selection() {
            const textarea = this.textarea;
            const start = textarea.selectionStart;
            const end = textarea.selectionEnd;

            if (start === end) return null;

            const value = textarea.value;
            const text = value.substring(start, end);
            const lines = this.getSelectedLines();

            return {
                text: text,
                selectionStart: start,
                selectionEnd: end,
                lines: lines
            };
        }

        getSelectedLines() {
            const textarea = this.textarea;
            const value = textarea.value;
            const start = textarea.selectionStart;
            const end = textarea.selectionEnd;

            const beforeSelection = value.substring(0, start);
            const lineStart = beforeSelection.lastIndexOf('\n') + 1;

            const afterSelection = value.substring(end);
            const nextNewline = afterSelection.indexOf('\n');
            const lineEnd = nextNewline === -1 ? value.length : end + nextNewline;

            const linesText = value.substring(lineStart, lineEnd);
            const allLines = value.split('\n');
            const startLine = beforeSelection.split('\n').length - 1;

            return linesText.split('\n').map((text, index) => ({
                text: text,
                lineNumber: startLine + index + 1
            }));
        }

        insert(text) {
            const textarea = this.textarea;
            const start = textarea.selectionStart;
            const end = textarea.selectionEnd;
            const value = textarea.value;

            const markerRegex = new RegExp(this.MARKER, 'g');
            const markers = text.match(markerRegex);
            const cleanText = text.replace(markerRegex, '');

            const newValue = value.substring(0, start) + cleanText + value.substring(end);
            textarea.value = newValue;

            if (markers && markers.length > 0) {
                const firstMarkerPos = start + text.indexOf(this.MARKER);
                const secondMarkerPos = markers.length > 1
                    ? start + text.lastIndexOf(this.MARKER) - this.MARKER.length
                    : firstMarkerPos;

                textarea.setSelectionRange(firstMarkerPos, secondMarkerPos);
            } else {
                textarea.setSelectionRange(start + cleanText.length, start + cleanText.length);
            }

            this.dispatchChange();
        }

        wrap(syntax, options = {}) {
            const { placeholder = 'text' } = options;
            const selection = this.selection;

            if (!selection || selection.text.trim() === '') {
                this.insert(`${syntax}${this.MARKER}${placeholder}${this.MARKER}${syntax}`);
            } else {
                const wrappedText = `${syntax}${selection.text}${syntax}`;
                this.replace(wrappedText);
            }
        }

        replace(text, selectReplaced = true) {
            const textarea = this.textarea;
            const start = textarea.selectionStart;
            const end = textarea.selectionEnd;
            const value = textarea.value;

            // Check for markers in the text
            const markerRegex = new RegExp(this.MARKER, 'g');
            const markers = text.match(markerRegex);
            const cleanText = text.replace(markerRegex, '');

            const newValue = value.substring(0, start) + cleanText + value.substring(end);
            textarea.value = newValue;

            if (markers && markers.length > 0) {
                // If there are markers, use them for selection
                const firstMarkerPos = start + text.indexOf(this.MARKER);
                const secondMarkerPos = markers.length > 1
                    ? start + text.lastIndexOf(this.MARKER) - this.MARKER.length
                    : firstMarkerPos;

                textarea.setSelectionRange(firstMarkerPos, secondMarkerPos);
            } else if (selectReplaced) {
                textarea.setSelectionRange(start, start + cleanText.length);
            } else {
                textarea.setSelectionRange(start + cleanText.length, start + cleanText.length);
            }

            this.dispatchChange();
        }

        replaceCurrentLines(callback) {
            const hadSelection = this.selection !== null;
            const originalStart = this.textarea.selectionStart;
            const originalEnd = this.textarea.selectionEnd;
            const lines = this.getSelectedLines();
            const newLines = lines.map((line, index) => callback(line, index));
            const newText = newLines.join('\n');

            this.replaceLines(newText, hadSelection, originalStart, originalEnd);
        }

        replaceLines(text, hadSelection = true, originalStart = null, originalEnd = null) {
            const textarea = this.textarea;
            const value = textarea.value;
            const start = textarea.selectionStart;
            const end = textarea.selectionEnd;

            const beforeSelection = value.substring(0, start);
            const lineStart = beforeSelection.lastIndexOf('\n') + 1;

            const afterSelection = value.substring(end);
            const nextNewline = afterSelection.indexOf('\n');
            const lineEnd = nextNewline === -1 ? value.length : end + nextNewline;

            const oldText = value.substring(lineStart, lineEnd);
            const newValue = value.substring(0, lineStart) + text + value.substring(lineEnd);
            textarea.value = newValue;

            if (hadSelection) {
                // If there was a selection, select the replaced text
                const newEnd = lineStart + text.length;
                textarea.setSelectionRange(lineStart, newEnd);
            } else {
                // If there was no selection, try to maintain cursor position relative to line start
                const cursorOffsetFromLineStart = originalStart - lineStart;
                const textLengthDiff = text.length - oldText.length;
                const newCursorPos = originalStart + textLengthDiff;
                textarea.setSelectionRange(newCursorPos, newCursorPos);
            }

            this.dispatchChange();
        }

        dispatchChange() {
            const event = new Event('input', { bubbles: true });
            this.textarea.dispatchEvent(event);
        }
    }

    class TextareaMarkdown {
        constructor(textarea, options = {}) {
            this.textarea = textarea;
            this.cursor = new Cursor(textarea);
            this.options = {
                preferredBoldSyntax: '**',
                preferredItalicSyntax: '*',
                preferredUnorderedListSyntax: '-',
                ...options
            };

            this.shortcuts = {
                'ctrl+b': () => this.bold(),
                'meta+b': () => this.bold(),
                'ctrl+i': () => this.italic(),
                'meta+i': () => this.italic(),
                'ctrl+shift+x': () => this.strikeThrough(),
                'meta+shift+x': () => this.strikeThrough(),
                'ctrl+k': () => this.link(),
                'meta+k': () => this.link(),
                'ctrl+shift+k': () => this.image(),
                'meta+shift+k': () => this.image(),
                'ctrl+shift+7': () => this.orderedList(),
                'meta+shift+7': () => this.orderedList(),
                'ctrl+shift+8': () => this.unorderedList(),
                'meta+shift+8': () => this.unorderedList(),
                'ctrl+e': () => this.codeInline(),
                'meta+e': () => this.codeInline(),
                'ctrl+shift+e': () => this.codeBlock(),
                'meta+shift+e': () => this.codeBlock(),
                'ctrl+shift+.': () => this.blockQuotes(),
                'meta+shift+.': () => this.blockQuotes(),
                'ctrl+alt+1': () => this.heading(1),
                'meta+alt+1': () => this.heading(1),
                'ctrl+alt+2': () => this.heading(2),
                'meta+alt+2': () => this.heading(2),
                'ctrl+alt+3': () => this.heading(3),
                'meta+alt+3': () => this.heading(3),
                'ctrl+alt+4': () => this.heading(4),
                'meta+alt+4': () => this.heading(4),
                'ctrl+alt+5': () => this.heading(5),
                'meta+alt+5': () => this.heading(5),
                'ctrl+alt+6': () => this.heading(6),
                'meta+alt+6': () => this.heading(6),
                'tab': () => this.indent(),
                'shift+tab': () => this.unindent()
            };

            this.init();
        }

        init() {
            this.textarea.addEventListener('keydown', (e) => this.handleKeydown(e));
            this.textarea.addEventListener('keydown', (e) => {
                if (e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) {
                    this.handleEnter(e);
                }
            });
            this.textarea.addEventListener('paste', (e) => this.handlePaste(e));
        }

        handleKeydown(e) {
            // const key = this.getKeyCombo(e);
            // const handler = this.shortcuts[key];

            // if (handler) {
            //     e.preventDefault();
            //     handler(e);
            // }
        }

        getKeyCombo(e) {
            const parts = [];

            if (e.ctrlKey) parts.push('ctrl');
            if (e.metaKey) parts.push('meta');
            if (e.altKey) parts.push('alt');
            if (e.shiftKey) parts.push('shift');

            // Normalize the key name to lowercase
            const key = e.key.toLowerCase();
            parts.push(key);

            return parts.join('+');
        }

        // Command implementations
        bold() {
            this.cursor.wrap(this.options.preferredBoldSyntax, { placeholder: 'bold' });
        }

        italic() {
            this.cursor.wrap(this.options.preferredItalicSyntax, { placeholder: 'italic' });
        }

        strikeThrough() {
            this.cursor.wrap('~~', { placeholder: 'strikethrough' });
        }

        link() {
            const selection = this.cursor.selection;
            if (!selection || selection.text.trim() === '') {
                this.cursor.insert(`[${this.cursor.MARKER}text${this.cursor.MARKER}](url)`);
            } else {
                const text = selection.text;
                const linkText = `[${text}](${this.cursor.MARKER}url${this.cursor.MARKER})`;
                this.cursor.replace(linkText, false);
            }
        }

        image() {
            const selection = this.cursor.selection;
            if (!selection || selection.text.trim() === '') {
                this.cursor.insert(`![${this.cursor.MARKER}alt text${this.cursor.MARKER}](image.png)`);
            } else {
                const text = selection.text;
                const imageText = `![${text}](${this.cursor.MARKER}image.png${this.cursor.MARKER})`;
                this.cursor.replace(imageText, false);
            }
        }

        codeInline() {
            this.cursor.wrap('`', { placeholder: 'code' });
        }

        codeBlock() {
            const selection = this.cursor.selection;
            if (!selection || selection.text.trim() === '') {
                this.cursor.insert('```\n' + this.cursor.MARKER + 'code' + this.cursor.MARKER + '\n```');
            } else {
                const text = selection.text;
                this.cursor.replace('```\n' + text + '\n```');
            }
        }

        orderedList() {
            const lines = this.cursor.getSelectedLines();
            const re = /^\d+\.\s/;
            const needUndo = lines.every(line => re.test(line.text));

            this.cursor.replaceCurrentLines((line, index) => {
                if (needUndo) {
                    return line.text.replace(re, '');
                } else {
                    return `${index + 1}. ${line.text}`;
                }
            });
        }

        unorderedList() {
            const syntax = this.options.preferredUnorderedListSyntax;
            const lines = this.cursor.getSelectedLines();
            const re = new RegExp(`^[\\-\\*\\+]\\s`);
            const needUndo = lines.every(line => re.test(line.text));

            this.cursor.replaceCurrentLines((line) => {
                if (needUndo) {
                    return line.text.replace(re, '');
                } else {
                    return `${syntax} ${line.text}`;
                }
            });
        }

        blockQuotes() {
            const lines = this.cursor.getSelectedLines();
            const re = /^>\s/;
            const needUndo = lines.every(line => re.test(line.text));

            this.cursor.replaceCurrentLines((line) => {
                if (needUndo) {
                    return line.text.replace(re, '');
                } else {
                    return `> ${line.text}`;
                }
            });
        }

        heading(level) {
            const lines = this.cursor.getSelectedLines();
            const prefix = '#'.repeat(level) + ' ';
            const re = new RegExp(`^#{1,6}\\s`);

            this.cursor.replaceCurrentLines((line) => {
                const text = line.text.replace(re, '');
                return prefix + text;
            });
        }

        indent() {
            // Always indent the current line(s) to the next multiple of 4
            this.cursor.replaceCurrentLines((line) => {
                const leadingSpaces = line.text.match(/^(\s*)/)[1].length;
                const spacesToAdd = 4 - (leadingSpaces % 4);
                return ' '.repeat(spacesToAdd) + line.text;
            });
        }

        unindent() {
            // Always unindent the current line(s) to the previous multiple of 4
            this.cursor.replaceCurrentLines((line) => {
                const leadingSpaces = line.text.match(/^(\s*)/)[1].length;
                if (leadingSpaces === 0) return line.text;

                const spacesToRemove = leadingSpaces % 4 === 0 ? 4 : leadingSpaces % 4;
                return line.text.substring(spacesToRemove);
            });
        }

        handleEnter(e) {
            const start = this.textarea.selectionStart;
            const value = this.textarea.value;
            const beforeCursor = value.substring(0, start);
            const lineStart = beforeCursor.lastIndexOf('\n') + 1;
            const currentLine = value.substring(lineStart, start);

            // Check for list patterns
            const orderedListMatch = currentLine.match(/^(\s*)(\d+)\.\s(.*)$/);
            const unorderedListMatch = currentLine.match(/^(\s*)([\-\*\+])\s(.*)$/);
            const blockQuoteMatch = currentLine.match(/^(\s*)(>)\s(.*)$/);

            if (orderedListMatch) {
                const [, indent, number, content] = orderedListMatch;

                // If the line is empty (just the marker)
                if (content.trim() === '') {
                    e.preventDefault();

                    // If there's indentation, remove to previous multiple of 4
                    if (indent.length > 0) {
                        const currentIndent = indent.length;
                        const spacesToRemove = currentIndent % 4 === 0 ? 4 : currentIndent % 4;
                        const newIndent = ' '.repeat(Math.max(0, currentIndent - spacesToRemove));

                        // Keep the same number when unindenting
                        const newValue = value.substring(0, lineStart) + newIndent + number + '. ' + value.substring(start);
                        this.textarea.value = newValue;
                        const cursorPos = lineStart + newIndent.length + number.length + 2;
                        this.textarea.setSelectionRange(cursorPos, cursorPos);
                        this.cursor.dispatchChange();
                    } else {
                        // No indentation, remove the list marker and create blank line
                        const newValue = value.substring(0, lineStart) + value.substring(start);
                        this.textarea.value = newValue;
                        this.textarea.setSelectionRange(lineStart, lineStart);
                        this.cursor.dispatchChange();
                    }
                } else {
                    // Continue the list
                    const nextNumber = parseInt(number) + 1;
                    e.preventDefault();
                    this.cursor.insert(`\n${indent}${nextNumber}. `);
                }
            } else if (unorderedListMatch) {
                const [, indent, marker, content] = unorderedListMatch;

                // If the line is empty (just the marker)
                if (content.trim() === '') {
                    e.preventDefault();

                    // If there's indentation, remove to previous multiple of 4
                    if (indent.length > 0) {
                        const currentIndent = indent.length;
                        const spacesToRemove = currentIndent % 4 === 0 ? 4 : currentIndent % 4;
                        const newIndent = ' '.repeat(Math.max(0, currentIndent - spacesToRemove));

                        const newValue = value.substring(0, lineStart) + newIndent + marker + ' ' + value.substring(start);
                        this.textarea.value = newValue;
                        this.textarea.setSelectionRange(lineStart + newIndent.length + 2, lineStart + newIndent.length + 2);
                        this.cursor.dispatchChange();
                    } else {
                        // No indentation, remove the list marker and create blank line
                        const newValue = value.substring(0, lineStart) + value.substring(start);
                        this.textarea.value = newValue;
                        this.textarea.setSelectionRange(lineStart, lineStart);
                        this.cursor.dispatchChange();
                    }
                } else {
                    // Continue the list
                    e.preventDefault();
                    this.cursor.insert(`\n${indent}${marker} `);
                }
            } else if (blockQuoteMatch) {
                const [, indent, content] = blockQuoteMatch;

                // If the line is empty (just the marker)
                if (content.trim() === '') {
                    e.preventDefault();

                    // If there's indentation, remove to previous multiple of 4
                    if (indent.length > 0) {
                        const currentIndent = indent.length;
                        const spacesToRemove = currentIndent % 4 === 0 ? 4 : currentIndent % 4;
                        const newIndent = ' '.repeat(Math.max(0, currentIndent - spacesToRemove));

                        const newValue = value.substring(0, lineStart) + newIndent + '> ' + value.substring(start);
                        this.textarea.value = newValue;
                        this.textarea.setSelectionRange(lineStart + newIndent.length + 2, lineStart + newIndent.length + 2);
                        this.cursor.dispatchChange();
                    } else {
                        // No indentation, remove the block quote marker and create blank line
                        const newValue = value.substring(0, lineStart) + value.substring(start);
                        this.textarea.value = newValue;
                        this.textarea.setSelectionRange(lineStart, lineStart);
                        this.cursor.dispatchChange();
                    }
                } else {
                    // Continue the block quote
                    e.preventDefault();
                    this.cursor.insert(`\n${indent}> `);
                }
            }
            // For regular text, don't prevent default - let browser handle it naturally
        }

        handlePaste(e) {
            const text = e.clipboardData.getData('text/plain');
            const urlPattern = /^https?:\/\/[^\s]+$/;

            if (urlPattern.test(text.trim())) {
                const selection = this.cursor.selection;
                if (selection && selection.text.trim()) {
                    e.preventDefault();
                    this.cursor.replace(`[${selection.text}](${text.trim()})`);
                }
            }
        }

        destroy() {
            // Remove event listeners if needed
        }
    }

    // Export for different module systems
    if (typeof module !== 'undefined' && module.exports) {
        module.exports = TextareaMarkdown;
    } else if (typeof define === 'function' && define.amd) {
        define([], function () {
            return TextareaMarkdown;
        });
    } else {
        window.TextareaMarkdown = TextareaMarkdown;
    }
})();