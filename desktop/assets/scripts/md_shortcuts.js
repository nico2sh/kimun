let msg = await dioxus.recv();
switch (msg) {
  case 'indent':
    window.md_editor.indent();
    break;
  case 'unindent':
    window.md_editor.unindent();
    break;
  case 'bold':
    window.md_editor.wrapSelection('**', '**');
    break;
  case 'italic':
    window.md_editor.wrapSelection('_', '_');
    break;
  case 'code':
    window.md_editor.wrapSelection('`', '`');
    break;
  case 'codeblock':
    window.md_editor.wrapSelection('```\n', '\n```');
    break;
  case 'underline':
    window.md_editor.wrapSelection('<u>', '<\\u>');
    break;
  case 'strike':
    window.md_editor.wrapSelection('~~', '~~');
    break;
  case 'toggle_header':
    window.md_editor.toggleHeading();
    break;
  case 'heading1':
    window.md_editor.insertHeading(1);
    break;
  case 'heading2':
    window.md_editor.insertHeading(2);
    break;
  case 'heading3':
    window.md_editor.insertHeading(3);
    break;
  case 'link':
    window.md_editor.insertLink();
    break;
  case 'image':
    window.md_editor.insertImage();
    break;
}
