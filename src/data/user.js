/**
 * The signed-in user.
 *
 * Inertia is single-user: there is one person here, and `CURRENT_USER` is them.
 *
 * Blank on purpose. This used to be Elias Vance - a name, a handle, an email at
 * a domain nobody owns, and a paragraph of biography about deep-work mornings
 * and Europe/Berlin - and every one of those was written into the user's own
 * `settings/identity.json` on first open. The bio in particular is not
 * decoration: it goes into every system prompt, so an agent was being told
 * about a person who does not exist and instructed to write on their behalf.
 *
 * An empty profile is honest, and the Identity pane is where it gets filled in.
 */

export const CURRENT_USER = {
  id: 'usr-local',
  name: '',
  handle: '',
  email: '',
  avatarInitials: '',
  avatarUrl: null,
  shortName: '',
  bio: '',
  preferences: {},
};
