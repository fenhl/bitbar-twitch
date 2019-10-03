use {
    std::vec,
    reqwest::{
        RequestBuilder,
        Response
    },
    serde::de::DeserializeOwned,
    serde_derive::Deserialize,
    crate::Error
};

#[derive(Debug, Default, Deserialize)]
struct Pagination {
    cursor: Option<String>
}

#[derive(Debug, Deserialize)]
struct PaginatedResponse<T> {
    total: Option<usize>,
    data: Vec<T>,
    #[serde(default)]
    pagination: Pagination
}

enum State {
    Start,
    Partial(usize),
    End
}

pub(crate) struct PaginatedList<T: DeserializeOwned> {
    request: RequestBuilder,
    cursor: Option<String>,
    cached_page: vec::IntoIter<T>,
    state: State
}

impl<T: DeserializeOwned> From<RequestBuilder> for PaginatedList<T> {
    fn from(request: RequestBuilder) -> PaginatedList<T> {
        PaginatedList {
            request,
            cursor: None,
            cached_page: Vec::default().into_iter(),
            state: State::Start
        }
    }
}

impl<T: DeserializeOwned> Iterator for PaginatedList<T> {
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Result<T, Error>> {
        // first, try to take the next item from the cached page, this works because vec::IntoIter implements FusedIterator
        if let Some(next_inner) = self.cached_page.next() {
            return Some(Ok(next_inner));
        }
        match self.state {
            State::Start => {}
            State::Partial(remaining) => if remaining == 0 {
                // the cache is empty and we've seen the expected number of items, so we're done
                self.state = State::End;
                return None;
            },
            State::End => { return None; }
        }
        // if the cache is empty and we haven't seen the end, download and cache the next page
        let mut request = match self.request.try_clone() {
            Some(request) => request,
            None => { return Some(Err(Error::UnclonableRequestBuilder)); }
        };
        request = request.query(&[("first", 100)]);
        if let Some(ref cursor) = self.cursor {
            request = request.query(&[("after", cursor)]);
        }
        let PaginatedResponse { total, data, pagination } = match request.send()
            .and_then(Response::error_for_status)
            .and_then(|mut response| response.json())
        {
            Ok(resp) => resp,
            Err(e) => { return Some(Err(e.into())); }
        };
        self.cursor = pagination.cursor;
        self.state = match self.state {
            State::Start => if let Some(total) = total {
                State::Partial(total - data.len())
            } else {
                // if no total has been sent, the response isn't paginated
                State::End
            },
            State::Partial(old_remaining) => State::Partial(old_remaining - data.len()),
            State::End => unreachable!()
        };
        self.cached_page = data.into_iter();
        // take the first element from the new page. If it's empty, we're done
        self.cached_page.next().map(Ok)
    }
}
