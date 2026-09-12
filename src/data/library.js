/**
 * The Library catalogue - every artifact the workspace has produced.
 *
 * What is left here is the vocabulary and nothing else: which tabs the screen
 * has and how each one is labelled and tinted. The artifacts themselves are no
 * longer a list in this file - they are derived from the transcripts by
 * `features/library/artifacts.js`, because an artifact IS a message's
 * attachment or a tool call's path, and a second copy of that could only ever
 * be wrong about one of them.
 *
 * Icons are glyph *names* resolved through `<Icon name>`.
 */

/** Category → how the screen labels and tints it. `conversation` is a tab, not a file. */
export const LIBRARY_CATEGORY_META = {
  conversation: { label: 'Conversations', icon: 'MessageCircle', tone: 'neutral' },
  image: { label: 'Images', icon: 'Image', tone: 'info' },
  document: { label: 'Documents', icon: 'FileText', tone: 'neutral' },
  data: { label: 'Data', icon: 'Table', tone: 'success' },
  audio: { label: 'Audio', icon: 'AudioLines', tone: 'warning' },
  video: { label: 'Video', icon: 'Video', tone: 'info' },
  code: { label: 'Code', icon: 'Code', tone: 'neutral' },
};

/** The tabs the Library shows, in order. `all` and `conversation` come first. */
export const LIBRARY_CATEGORIES = ['image', 'document', 'data', 'audio', 'video', 'code'];
