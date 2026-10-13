I would like to avoid shipping these library files together
with my application's source.

However, the official website is sometimes down, and they have also started
implementing measures to prevent automatic downloads and builds.

So, these files are currently included with the application's
source to prevent build errors.

Certain files which are not needed were removed from the libraries' archives
to save on space.

Feel free to contact me if you would like the files to be removed,
if an agreement about their availability can be reached.

build.rs extracts FTD3XX.dll and ftd2xx.dll from these archives and places
them next to the built executable.
