//! The QuickTime Movie Toolbox calls a game uses to play its movies (`Apple;Carbon;Multimedia` in a Carbon application,
//! `QuickTimeLib` in a classic one), as a native library with NO MOVIE DECODER. The calls exist and answer as the Movie Toolbox
//! does when a movie cannot be had: `EnterMovies` succeeds (the toolbox is there), every way of making a movie
//! (`NewMovie`, `NewMovieFromFile`, `MovieImportFile`) is refused by NAME on the log with `invalidMovie` (-2010) from
//! `GetMoviesError` and a NIL movie, every call on a NIL or unknown movie sets `invalidMovie` and changes nothing, and
//! `IsMovieDone` answers TRUE (there is nothing left to play). A game that checks for QuickTime then skips its movies the way it
//! skips a movie file it cannot open. Gestalt `'qtim'` names the API level these calls follow (QuickTime 4.0, gestalt_ppc.rs),
//! under the same ruling as `'sysv'` / `'cbon'`: the version selector names the personality, every missing piece is refused per
//! call by name. NAMED LIMIT: no movie is ever decoded or shown (a decoder is media.engine's, not this engine's).
//! Signatures: Movies.h (QuickTime 4 Universal Interfaces), from the public API contract.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_ppc::Cpu;

/// The library names a program imports the Movie Toolbox from.
pub fn provided(lib: &str) -> bool {
    matches!(lib, "Apple;Carbon;Multimedia" | "QuickTimeLib")
}

/// `invalidMovie`: the movie is NIL / not a movie (Movies.h error).
pub const INVALID_MOVIE: i32 = -2010;
/// `gestaltQuickTimeVersion` ('qtim'): QuickTime 4.0 final as a NumVersion - the API level of the calls served here.
pub const QUICKTIME_4: u32 = 0x0400_8000;

#[derive(Default)]
pub struct QtState {
    last_err: i32,
    refusals: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QtFn {
    EnterMovies, ExitMovies, GetMoviesError, ClearMoviesStickyError, NewMovie, NewMovieFromFile, MovieImportFile, DisposeMovie,
    GetMovieBox, SetMovieBox, SetMovieGWorld, StartMovie, StopMovie, MoviesTask, IsMovieDone, GoToBeginningOfMovie, SetMovieVolume,
}

impl QtFn {
    pub fn by_name(name: &str) -> Option<QtFn> {
        use QtFn::*;
        Some(match name {
            "EnterMovies" => EnterMovies, "ExitMovies" => ExitMovies, "GetMoviesError" => GetMoviesError,
            "ClearMoviesStickyError" => ClearMoviesStickyError, "NewMovie" => NewMovie, "NewMovieFromFile" => NewMovieFromFile,
            "MovieImportFile" => MovieImportFile, "DisposeMovie" => DisposeMovie, "GetMovieBox" => GetMovieBox, "SetMovieBox" => SetMovieBox,
            "SetMovieGWorld" => SetMovieGWorld, "StartMovie" => StartMovie, "StopMovie" => StopMovie, "MoviesTask" => MoviesTask,
            "IsMovieDone" => IsMovieDone, "GoToBeginningOfMovie" => GoToBeginningOfMovie, "SetMovieVolume" => SetMovieVolume,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn qt_call(&mut self, f: QtFn, cpu: &mut Cpu) -> Result<(), String> {
        use QtFn::*;
        let a0 = arg(self, cpu, 0)?;
        let (ret, err, line): (u32, i32, Option<String>) = match f {
            EnterMovies => {
                (0, 0, Some(String::from("[qt] EnterMovies: the Movie Toolbox is there; it has no movie decoder (every movie is refused by name)")))
            }
            ExitMovies => (0, 0, None),
            GetMoviesError => (self.qt.last_err as u32, self.qt.last_err, None),
            ClearMoviesStickyError => (0, self.qt.last_err, None),
            // Movie NewMovie(long); OSErr NewMovieFromFile(Movie *, short, short *, StringPtr, short, Boolean *); ComponentResult
            // MovieImportFile(MovieImportComponent, const FSSpec *, Movie, ...): no movie can be made.
            NewMovie | NewMovieFromFile | MovieImportFile => {
                self.qt.refusals += 1;
                if f == NewMovieFromFile && a0 != 0 {
                    let _ = self.mac.mem.write_u32(a0, 0); // *theMovie = NIL
                }
                let ret = if f == NewMovie { 0 } else { INVALID_MOVIE as u32 };
                (ret, INVALID_MOVIE, Some(format!("[qt] {f:?}: refused - this engine has no movie decoder (invalidMovie); the program goes on without the movie [QT-MOVIE REFUSED n={}]", self.qt.refusals)))
            }
            // Boolean IsMovieDone(Movie): nothing is playing, so it is done.
            IsMovieDone => (1, INVALID_MOVIE, None),
            // void calls on a movie: none exists, so each is invalidMovie and changes nothing (GetMovieBox leaves the Rect as it was).
            DisposeMovie | GetMovieBox | SetMovieBox | SetMovieGWorld | StartMovie | StopMovie | MoviesTask | GoToBeginningOfMovie | SetMovieVolume => {
                (0, INVALID_MOVIE, None)
            }
        };
        if !matches!(f, GetMoviesError | ClearMoviesStickyError) {
            self.qt.last_err = err;
        } else if f == ClearMoviesStickyError {
            self.qt.last_err = 0;
        }
        if let Some(l) = line {
            self.mac.log.push(l);
        }
        cpu.gpr[3] = ret;
        Ok(())
    }
}
