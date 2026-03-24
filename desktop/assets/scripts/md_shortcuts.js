let msg = await dioxus.recv();
switch (msg) {
  case 'indent':
    window.editor.indent();
    break;
  case 'unindent':
    window.editor.unindent();
    break;
  case 'bold':
    window.editor.bold();
    break;
  case 'italic':
    window.editor.italic();
    break;
  case 'code':
    window.editor.codeInline();
    break;
  case 'codeblock':
    window.editor.codeBlock();
    break;
  case 'quote':
    window.editor.blockQuotes();
    break;
  // case 'underline':
  //   window.md_editor.wrapSelection('<u>', '<\\u>');
  //   break;
  case 'strike':
    window.editor.strikeThrough();
    break;
  case 'toggle_header':
    window.editor.heading(1);
    break;
  case 'heading1':
    window.editor.heading(1);
    break;
  case 'heading2':
    window.editor.heading(2);
    break;
  case 'heading3':
    window.editor.heading(3);
    break;
  case 'link':
    window.editor.link();
    break;
  case 'image':
    window.editor.image();
    break;
}
